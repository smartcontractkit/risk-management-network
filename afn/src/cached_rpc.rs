use anyhow::Context;
use minieth::keccak::keccak256;
use rusqlite::{named_params, OptionalExtension};

use std::{
    cell::RefCell,
    cmp::{max, min},
    fs::{self, DirEntry},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use minieth::{
    bytes::{Address, Bytes32},
    rpc::{EVMLog, Rpc},
};

use crate::{common::ChainName, config::ChainStability, smart_get_logs::SmartGetLogs};

const CACHE_DIR: &str = "cache";

const SCHEMA_VERSION: u64 = 4;

pub struct CachedRpc {
    parent_worker_name: String,
    db: RefCell<LogsDb>,
    smart_get_logs: SmartGetLogs,
}

fn query_shape_digest(
    chain: ChainName,
    chain_stability: ChainStability,
    addresses: &[Address],
    first_topics: &[Bytes32],
) -> anyhow::Result<Bytes32> {
    let mut addrs_buf = Vec::with_capacity(std::mem::size_of_val(addresses));
    for addr in addresses {
        addrs_buf.extend_from_slice(addr);
    }
    let mut first_topics_buf = Vec::with_capacity(std::mem::size_of_val(first_topics));
    for first_topic in first_topics {
        first_topics_buf.extend_from_slice(first_topic);
    }
    let mut final_buf =
        Vec::with_capacity(std::mem::size_of::<u64>() + 3 * std::mem::size_of::<Bytes32>());
    final_buf.extend_from_slice(&chain.chain_id().to_be_bytes());
    final_buf.extend_from_slice(&keccak256(&serde_json::to_vec(&chain_stability)?));
    final_buf.extend_from_slice(&keccak256(&addrs_buf));
    final_buf.extend_from_slice(&keccak256(&first_topics_buf));
    Ok(keccak256(&final_buf).into())
}

struct LogsDb(rusqlite::Connection);

const MAX_NUM_LOGS_OVER_MANY_BLOCKS: u64 = 1_500;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PartialLogs {
    pub to_block: u64,
    pub logs: Vec<EVMLog>,
}

impl LogsDb {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, rusqlite::Error> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(
            "
        create table if not exists logs (
            block_number int not null,
            block_hash blob not null,
            address blob not null,
            log_index int not null,
            topic0 blob not null, topic1 blob, topic2 blob, topic3 blob,
            data blob not null,
            unique(block_number, log_index)
        );

        create table if not exists queriable_block_ranges (from_block int not null, to_block int not null);
        create index if not exists queriable_block_ranges_covering_from_to on queriable_block_ranges(from_block, to_block);
        ",
        )?;
        Ok(Self(conn))
    }

    fn get_partial_logs(
        &mut self,
        from_block: u64,
        to_block: u64,
    ) -> anyhow::Result<Option<PartialLogs>> {
        anyhow::ensure!(
            from_block <= to_block,
            "from_block {from_block} must be <= to_block {to_block}",
        );

        let tx = self.0.transaction()?;
        let db_to_block = match tx.query_row(
            "select to_block from queriable_block_ranges
            where from_block <= :from_block and to_block >= :from_block
            order by to_block desc limit 1",
            named_params! { ":from_block": from_block, },
            |row| row.get(0),
        ) {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        let mut to_block = std::cmp::min(db_to_block, to_block);
        loop {
            let from_block_to_block_params = named_params! {
                ":from_block": from_block,
                ":to_block": to_block,
            };

            let num_logs: u64 = tx.query_row(
                "select count(1) from logs
                where block_number >= :from_block and block_number <= :to_block",
                from_block_to_block_params,
                |row| row.get(0),
            )?;

            if from_block < to_block && num_logs > MAX_NUM_LOGS_OVER_MANY_BLOCKS {
                to_block = from_block + (to_block - from_block) / 2;
                continue;
            }

            let mut st = tx.prepare_cached(
                "select block_number, block_hash, address, log_index, topic0, topic1, topic2, topic3, data
                from logs
                where block_number >= :from_block and block_number <= :to_block
                order by block_number, log_index asc",
            )?;
            let log_results = st.query_map(from_block_to_block_params, evm_log_from_row)?;
            let logs = log_results.collect::<Result<Vec<EVMLog>, rusqlite::Error>>()?;
            break Ok(Some(PartialLogs { to_block, logs }));
        }
    }

    pub fn save_logs(
        &mut self,
        logs: &[EVMLog],
        from_block: u64,
        to_block: u64,
    ) -> Result<(), rusqlite::Error> {
        let tx = self.0.transaction()?;
        {
            let min_from_block = tx
                .query_row(
                    "select from_block from queriable_block_ranges
                    where to_block+1 >= :from_block_new order by from_block asc limit 1",
                    named_params! { ":from_block_new": from_block, },
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(from_block);

            let max_to_block = tx
                .query_row(
                    "select to_block from queriable_block_ranges
                    where from_block-1 <= :to_block_new order by to_block desc limit 1",
                    named_params! { ":to_block_new": to_block, },
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(to_block);

            tx.execute("delete from queriable_block_ranges where from_block >= :min_from_block and to_block <= :max_to_block", named_params! {
                ":min_from_block": min_from_block,
                ":max_to_block": max_to_block,
            })?;
            tx.execute("insert into queriable_block_ranges (from_block, to_block) values (:min_from_block, :max_to_block)", named_params! {
                ":min_from_block": min(from_block, min_from_block),
                ":max_to_block": max(to_block, max_to_block),
            })?;
        }
        {
            let mut st = tx
            .prepare_cached(
                "insert or replace into logs (
            block_number, block_hash, address, log_index, topic0, topic1, topic2, topic3, data
        ) values (
            :block_number, :block_hash, :address, :log_index, :topic0, :topic1, :topic2, :topic3, :data
        )",
            )?;
            for log in logs {
                #[allow(clippy::get_first)]
                st.execute(named_params! {
                    ":block_number": log.block_number,
                    ":block_hash": log.block_hash.as_slice(),
                    ":address": log.address.as_slice(),
                    ":log_index": log.log_index,
                    ":topic0": log.topics.get(0).map(Bytes32::as_slice),
                    ":topic1": log.topics.get(1).map(Bytes32::as_slice),
                    ":topic2": log.topics.get(2).map(Bytes32::as_slice),
                    ":topic3": log.topics.get(3).map(Bytes32::as_slice),
                    ":data": log.data.as_ref(),
                })?;
            }
        }
        tx.commit()
    }
}

fn evm_log_from_row(r: &rusqlite::Row) -> Result<EVMLog, rusqlite::Error> {
    let block_number = r.get("block_number")?;
    let log_index = r.get("log_index")?;
    let block_hash: [u8; 32] = r.get("block_hash")?;
    let address: [u8; 20] = r.get("address")?;
    let topic0: Option<[u8; 32]> = r.get("topic0")?;
    let topic1: Option<[u8; 32]> = r.get("topic1")?;
    let topic2: Option<[u8; 32]> = r.get("topic2")?;
    let topic3: Option<[u8; 32]> = r.get("topic3")?;
    let data: Vec<u8> = r.get("data")?;
    let all_topics = [topic0, topic1, topic2, topic3];
    let topics = all_topics
        .into_iter()
        .flatten()
        .map(Bytes32::from)
        .collect();
    Ok(EVMLog {
        log_index,
        address: address.into(),
        data: data.into(),
        topics,
        block_hash: block_hash.into(),
        block_number,
    })
}

fn str_to_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() => c,
            _ => '_',
        })
        .collect()
}

fn get_or_migrate_db_file_path(
    chain: ChainName,
    chain_stability: ChainStability,
    addresses: &[Address],
    first_topics: &[Bytes32],
    parent_worker_name: &str,
    cache_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let query_shape_digest = query_shape_digest(chain, chain_stability, addresses, first_topics)?;
    let expected_db_filename_suffix = format!("_{}", query_shape_digest);
    let mut existing_db_file: Option<DirEntry> = None;
    for file in fs::read_dir(cache_dir)? {
        let file = file?;
        if !file.path().is_file() {
            continue;
        }
        if file
            .file_name()
            .to_str()
            .ok_or(anyhow::anyhow!(
                "database filename is not valid unicode {file:?}"
            ))?
            .ends_with(&expected_db_filename_suffix)
        {
            if let Some(ref existing_db_file) = existing_db_file {
                if existing_db_file.path() == file.path() {
                    continue;
                }
                anyhow::bail!(
                    "found multiple matching databases for worker {}: {}, {}",
                    parent_worker_name,
                    existing_db_file.path().display(),
                    file.path().display(),
                )
            } else {
                tracing::info!(
                    "{}: existing db file found: {}",
                    parent_worker_name,
                    file.path().display()
                );
                existing_db_file = Some(file);
            }
        }
    }
    let desired_db_file_path = {
        let file_name = format!(
            "{}_{}",
            str_to_filename(parent_worker_name),
            query_shape_digest,
        );
        cache_dir.join(file_name)
    };
    if let Some(existing_db_file) = existing_db_file {
        let existing_db_file_path = existing_db_file.path();
        if existing_db_file_path != desired_db_file_path {
            tracing::info!(
                "{}: renaming db file from {} to {}",
                parent_worker_name,
                existing_db_file_path.display(),
                desired_db_file_path.display(),
            );
            fs::rename(existing_db_file_path, &desired_db_file_path)?;
        }
    } else {
        tracing::info!(
            "{}: no existing db file found, using new db file {}",
            parent_worker_name,
            desired_db_file_path.display(),
        )
    }
    Ok(desired_db_file_path)
}

impl CachedRpc {
    pub fn new(
        chain: ChainName,
        addresses: &[Address],
        first_topics: Vec<Bytes32>,
        rpc: Arc<Rpc>,
        chain_stability: ChainStability,
        parent_worker_name: &str,
    ) -> anyhow::Result<Self> {
        let cache_dir = {
            let dir = {
                let base_dir = std::env::current_dir()?;
                base_dir.join(CACHE_DIR).join(format!("v{SCHEMA_VERSION}"))
            };
            fs::create_dir_all(&dir)
                .with_context(|| format!("failed to create cache directory {}", dir.display()))?;
            dir
        };
        let db_file_path = get_or_migrate_db_file_path(
            chain,
            chain_stability,
            addresses,
            &first_topics,
            parent_worker_name,
            &cache_dir,
        )?;
        tracing::debug!("{parent_worker_name}: db in {db_file_path:?}");
        let db = LogsDb::open(db_file_path).context("failed to open logs db")?;

        let addresses = addresses.to_vec();

        Ok(Self {
            parent_worker_name: parent_worker_name.to_owned(),
            db: db.into(),
            smart_get_logs: SmartGetLogs::new(rpc, addresses, first_topics),
        })
    }

    fn try_save_to_cache(&self, logs: &[EVMLog], from_block: u64, to_block: u64) {
        let start = Instant::now();
        match self.db.borrow_mut().save_logs(logs, from_block, to_block) {
            Ok(_) => {
                tracing::debug!(
                    from_block,
                    to_block,
                    elapsed=?start.elapsed(),
                    "{}: wrote to cache",
                    self.parent_worker_name,
                );
            }
            Err(e) => {
                tracing::warn!(
                    from_block,
                    to_block,
                    elapsed=?start.elapsed(),
                    error=?e,
                    "{}: failed to write to cache",
                    self.parent_worker_name,
                );
            }
        }
    }

    fn try_fetch_from_cache(&self, from_block: u64, to_block: u64) -> Option<PartialLogs> {
        let start = Instant::now();
        match self.db.borrow_mut().get_partial_logs(from_block, to_block) {
            Ok(Some(cached_logs)) => {
                tracing::debug!(
                    from_block,
                    to_block=cached_logs.to_block,
                    max_to_block=to_block,
                    elapsed=?start.elapsed(),
                    num_logs=cached_logs.logs.len(),
                    "{}: cache hit",
                    self.parent_worker_name,
                );
                Some(cached_logs)
            }
            Ok(None) => {
                tracing::trace!(
                    from_block,
                    to_block,
                    "{}: cache miss",
                    self.parent_worker_name
                );
                None
            }
            Err(e) => {
                tracing::warn!(
                    from_block,
                    to_block,
                    elapsed=?start.elapsed(),
                    error=?e,
                    "{}: cache error",
                    self.parent_worker_name,
                );
                None
            }
        }
    }

    pub fn get_partial_logs(
        &self,
        from_block: u64,
        to_block: u64,
        cache: bool,
    ) -> anyhow::Result<PartialLogs> {
        anyhow::ensure!(
            from_block <= to_block,
            "from_block {from_block} must be <= to_block {to_block}",
        );

        if cache {
            if let Some(partial_logs) = self.try_fetch_from_cache(from_block, to_block) {
                return Ok(partial_logs);
            }
        }

        let partial_logs = self.smart_get_logs.get_partial_logs(from_block, to_block)?;

        if cache {
            self.try_save_to_cache(&partial_logs.logs, from_block, partial_logs.to_block);
        }

        Ok(partial_logs)
    }
}
