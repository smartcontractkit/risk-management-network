use std::{
    borrow::Cow,
    fs::{self, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    str::FromStr,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::Context;

const COMPRESSED_LOG_SUFFIX: &str = ".log.zst";

struct WriterWithMetadata<W: io::Write> {
    inner: W,
    pub creation: Instant,
    pub bytes_written: usize,
}

impl<W: io::Write> WriterWithMetadata<W> {
    fn new(inner: W) -> Self {
        WriterWithMetadata {
            inner,
            creation: Instant::now(),
            bytes_written: 0,
        }
    }
}

impl<W: io::Write> io::Write for WriterWithMetadata<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let bytes_written = self.inner.write(buf)?;
        self.bytes_written += bytes_written;
        Ok(bytes_written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub struct LogRotateConfig {
    pub root_dir: Cow<'static, str>,
    pub rotatable_log_file_age: Duration,
    pub rotatable_uncompressed_log_file_size: usize,
    pub reapable_log_file_age: Duration,
    pub failed_rotate_cooldown: Duration,
}

pub struct LogRotate<'zstd> {
    config: LogRotateConfig,
    root_dir: PathBuf,

    rotate_allowed_after: Instant,

    reap_join_handle: Option<JoinHandle<()>>,

    zstd: WriterWithMetadata<zstd::stream::AutoFinishEncoder<'zstd, BufWriter<fs::File>>>,
}

enum LogType {
    Initial,
    Continuing,
}

impl<'zstd> LogRotate<'zstd> {
    fn open_zstd(
        path: &Path,
    ) -> io::Result<WriterWithMetadata<zstd::stream::AutoFinishEncoder<'zstd, BufWriter<fs::File>>>>
    {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let buf_file = BufWriter::new(file);
        Ok(WriterWithMetadata::new(
            zstd::Encoder::new(buf_file, 0)?.auto_finish(),
        ))
    }

    fn new_filename(log_type: LogType) -> String {
        format!(
            "{:?}{}{}",
            chrono::Utc::now(),
            match log_type {
                LogType::Initial => "",
                LogType::Continuing => "-contd",
            },
            COMPRESSED_LOG_SUFFIX,
        )
    }

    pub fn new(config: LogRotateConfig) -> anyhow::Result<Self> {
        let root_dir = PathBuf::from_str(&config.root_dir)?;
        std::fs::create_dir_all(&root_dir)?;
        reap_logs(root_dir.clone(), config.reapable_log_file_age);

        let path = root_dir.join(Self::new_filename(LogType::Initial));
        let zstd = Self::open_zstd(&path)?;
        eprintln!("writing forensics logs to {:?}", path);
        Ok(LogRotate {
            config,
            root_dir,
            zstd,
            rotate_allowed_after: Instant::now(),
            reap_join_handle: None,
        })
    }

    fn maybe_rotate(&mut self) -> io::Result<()> {
        if self.zstd.creation.elapsed() < self.config.rotatable_log_file_age
            && self.zstd.bytes_written < self.config.rotatable_uncompressed_log_file_size
        {
            return Ok(());
        }

        if Instant::now() <= self.rotate_allowed_after {
            return Ok(());
        }

        self.zstd.flush()?;

        if let Some(join_handle) = self.reap_join_handle.take() {
            let _ = join_handle.join();
        }
        self.reap_join_handle = Some(std::thread::spawn({
            let root_dir = self.root_dir.clone();
            let reapable_log_file_age = self.config.reapable_log_file_age;
            move || reap_logs(root_dir, reapable_log_file_age)
        }));

        let new_path = self.root_dir.join(Self::new_filename(LogType::Continuing));
        let new_zstd = Self::open_zstd(&new_path)?;
        eprintln!("now writing forensics logs to {:?}", new_path);
        self.zstd = new_zstd;
        Ok(())
    }
}

impl io::Write for LogRotate<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.maybe_rotate().unwrap_or_else(|e| {
            eprintln!("⚠️ failed to rotate forensic logs: {:?}", e);
            self.rotate_allowed_after = Instant::now() + self.config.failed_rotate_cooldown;
        });
        self.zstd.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.zstd.flush()
    }
}

fn reap_logs(root_dir: PathBuf, reapable_log_file_age: Duration) {
    let result = fs::read_dir(root_dir).map(|dir| {
        dir.map(|entry| -> anyhow::Result<Option<PathBuf>> {
            let entry = entry.context("while opening direntry")?;
            let path = entry.path();
            if path.is_file()
                && path
                    .file_name()
                    .map(|file_name| file_name.to_string_lossy().ends_with(COMPRESSED_LOG_SUFFIX))
                    .unwrap_or(false)
            {
                let age = entry
                    .metadata()
                    .with_context(|| format!("could not get metadata for {path:?}"))?
                    .modified()
                    .with_context(|| format!("could not get modified time for {path:?}"))?
                    .elapsed()
                    .with_context(|| format!("could not get time since modified for {path:?}"))?;
                Ok(if age > reapable_log_file_age {
                    Some(path)
                } else {
                    None
                })
            } else {
                Ok(None)
            }
        })
        .collect::<Vec<_>>()
    });

    match result {
        Ok(files) => {
            files.into_iter().for_each(|file| match file {
                Ok(Some(path)) => {
                    let remove_result = fs::remove_file(&path);
                    eprintln!("🪓 reaping forensic log {:?}: {:?}", path, remove_result);
                }
                Ok(None) => {}
                Err(e) => eprintln!("⚠️ failed to check file: {:?}", e),
            });
        }
        Err(e) => {
            eprintln!("‼️ failed to read forensics directory: {:?}", e);
        }
    }
}
