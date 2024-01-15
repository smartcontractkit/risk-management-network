use anyhow::Result;
use std::{
    borrow::Cow,
    sync::Mutex,
    sync::{Arc, Condvar},
    thread::{self, JoinHandle},
    time::Duration,
};
use tracing::debug;

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

#[derive(Debug, Default)]
pub struct Context {
    done: Mutex<bool>,
    cvar: Condvar,
}

impl Context {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
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

    pub fn repeat<F>(&self, duration: Duration, mut f: F) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()> + 'static,
    {
        let mut done = self.is_done();
        while !done {
            f(self)?;
            (done, _) = self.sleep(duration);
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
        duration: Duration,
        f: F,
    ) -> ShutdownHandle
    where
        F: FnMut(&Self) -> Result<()> + Send + 'static,
    {
        Self::spawn(self, name, move |ctx, _name| ctx.repeat(duration, f))
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
