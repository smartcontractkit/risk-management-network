use anyhow::Result;
use std::{
    borrow::Cow,
    cmp::{max, min},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tracing::debug;

use crate::metrics::WorkerMetrics;

#[derive(Debug)]
pub struct ShutdownHandle {
    ctx: Arc<Context>,
    worker_name: String,
    join_handle: Option<JoinHandle<Result<()>>>,
}

impl ShutdownHandle {
    pub fn new(
        ctx: Arc<Context>,
        worker_name: String,
        join_handle: JoinHandle<Result<()>>,
    ) -> Self {
        Self {
            ctx,
            worker_name,
            join_handle: Some(join_handle),
        }
    }

    pub fn is_finished(&self) -> bool {
        self.join_handle
            .as_ref()
            .map(|j| j.is_finished())
            .unwrap_or(true)
    }
    fn try_join(&mut self) -> Result<()> {
        tracing::debug!("trying to join {}", self.worker_name);
        let result = match self.join_handle.take() {
            None => Err(anyhow::anyhow!("already joined")),
            Some(join_handle) => match join_handle.join() {
                Err(e) => Err(anyhow::anyhow!("panicked: {:?}", e)),
                Ok(Err(e)) => Err(anyhow::anyhow!("errored: {:?}", e)),
                Ok(Ok(())) => Ok(()),
            },
        };
        match &result {
            Err(e) => {
                tracing::error!("worker {} {:?}", self.worker_name, e);
            }
            Ok(()) => {
                tracing::info!("worker {} joined normally", self.worker_name);
            }
        };
        result
    }
}

impl Drop for ShutdownHandle {
    fn drop(&mut self) {
        self.ctx.cancel();
        let _join_result = self.try_join();
    }
}

#[derive(Debug, Default)]
pub struct ShutdownHandleGroup {
    pub handles: Vec<ShutdownHandle>,
}

impl ShutdownHandleGroup {
    pub fn add<T>(&mut self, (ret, handle): (T, ShutdownHandle)) -> T {
        self.handles.push(handle);
        ret
    }

    pub fn add_group<T>(&mut self, (ret, mut other): (T, ShutdownHandleGroup)) -> T {
        self.handles.append(&mut other.handles);
        ret
    }

    pub fn finished_workers_and_results(&mut self) -> Vec<(String, anyhow::Result<()>)> {
        let finished_worker = self
            .handles
            .iter_mut()
            .filter_map(|handle| {
                if handle.is_finished() {
                    Some((handle.worker_name.clone(), handle.try_join()))
                } else {
                    None
                }
            })
            .collect();
        finished_worker
    }
}

#[derive(Debug)]
pub struct Context {
    pub worker_metrics: Arc<Mutex<WorkerMetrics>>,

    done: Mutex<bool>,
    cvar: Condvar,
}

const MIN_POLL_DELAY_AFTER_ERROR: Duration = Duration::from_secs(1);
const MAX_POLL_DELAY_AFTER_ERROR: Duration = Duration::from_secs(5 * 60);
const DELAY_MULTIPLIER_AFTER_ERROR: u32 = 2;

impl Context {
    pub fn new() -> Arc<Self> {
        Arc::new(Context {
            worker_metrics: Arc::new(Mutex::new(WorkerMetrics::new())),
            done: Default::default(),
            cvar: Default::default(),
        })
    }
    pub fn cancel(&self) {
        let mut done = self.done.lock().unwrap();
        *done = true;
        self.cvar.notify_all();
    }

    pub fn is_done(&self) -> bool {
        let guard = self.done.lock().unwrap();
        *guard
    }

    pub fn sleep(&self, duration: Duration) -> (bool, bool) {
        let guard = self.done.lock().unwrap();
        let mut done = *guard;

        if done {
            return (done, false);
        }

        let (guard, result) = self
            .cvar
            .wait_timeout_while(guard, duration, |done| !*done)
            .unwrap();
        done = *guard;
        (done, result.timed_out())
    }

    pub fn repeat<F>(
        &self,
        name: impl Into<Cow<'static, str>>,
        min_interval: Duration,
        mut f: F,
    ) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()> + 'static,
    {
        let worker_name = name.into();
        let worker_metrics_handle = self
            .worker_metrics
            .clone()
            .lock()
            .unwrap()
            .make_handle(worker_name.to_string());

        let mut next_poll_delay = min_interval;
        let mut consecutive_errors = 0;
        let mut done = self.is_done();
        while !done {
            let poll_start = Instant::now();
            let poll_result = f(self);
            worker_metrics_handle
                .last_poll_duration_seconds
                .set(poll_start.elapsed().as_secs_f64());
            match poll_result {
                Ok(()) => {
                    next_poll_delay = min_interval;
                    consecutive_errors = 0;
                }
                Err(e) => {
                    next_poll_delay = min(
                        max(
                            MIN_POLL_DELAY_AFTER_ERROR,
                            next_poll_delay.saturating_mul(DELAY_MULTIPLIER_AFTER_ERROR),
                        ),
                        MAX_POLL_DELAY_AFTER_ERROR,
                    );
                    worker_metrics_handle.worker_errors.inc();
                    consecutive_errors += 1;
                    tracing::error!(
                        consecutive_errors,
                        "{worker_name}: worker poll failed: {e:?}; retrying in {next_poll_delay:?}"
                    );
                }
            }
            worker_metrics_handle
                .next_poll_delay_seconds
                .set(next_poll_delay.as_secs() as f64);
            worker_metrics_handle
                .consecutive_worker_errors
                .set(consecutive_errors as f64);
            (done, _) = self.sleep(next_poll_delay);
        }
        Ok(())
    }

    pub fn spawn<F>(self: &Arc<Self>, name: impl Into<Cow<'static, str>>, f: F) -> ShutdownHandle
    where
        F: FnOnce(Arc<Self>, &str) -> Result<()> + Send + 'static,
    {
        let ctx = self.clone();
        let name: Cow<'static, str> = name.into();
        let worker_name = name.to_string();

        let handle = std::thread::Builder::new()
            .name(worker_name.clone())
            .spawn(move || {
                debug!("{name}: spawned");
                let res = f(ctx, &name);
                debug!("{name}: stopped cleanly");
                res
            })
            .expect("failed to spawn thread");
        ShutdownHandle::new(Arc::clone(self), worker_name, handle)
    }

    pub fn spawn_repeat<F>(
        self: &Arc<Self>,
        name: impl Into<Cow<'static, str>>,
        min_interval: Duration,
        f: F,
    ) -> ShutdownHandle
    where
        F: FnMut(&Self) -> Result<()> + Send + 'static,
    {
        let name: Cow<'static, str> = name.into();
        let worker_name = name.to_string();
        Self::spawn(self, name, move |ctx, _name| {
            ctx.repeat(worker_name, min_interval, f)
        })
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        self.cancel()
    }
}

pub fn worker_name() -> String {
    match thread::current().name() {
        Some(name) => name.to_owned(),
        None => "[unnamed worker]".to_owned(),
    }
}
