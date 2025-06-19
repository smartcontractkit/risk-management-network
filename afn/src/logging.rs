use {
    crate::forensics::LogRotate,
    anyhow::{bail, Result},
    std::{env, sync::Mutex},
};

const LOG_JSON_ENV_VAR: &str = "AFN_LOG_JSON";

pub fn setup_tracing() -> Result<()> {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{filter, filter::LevelFilter, EnvFilter};

    let forensics_layer = tracing_subscriber::fmt::layer()
        .with_writer(Mutex::new(LogRotate::new(
            crate::config::FORENSICS_LOGROTATE_CONFIG,
        )?))
        .json()
        .flatten_event(true)
        .with_filter(filter::Targets::new().with_targets(vec![
            (
                minieth::json_rpc::LOG_SPAM_FORENSICS_TARGET,
                LevelFilter::TRACE,
            ),
            ("", LevelFilter::DEBUG),
        ]));

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    let wrap_tracing_error = |e| anyhow::anyhow!("failed to initialize tracing: {e:?}");

    match env::var(LOG_JSON_ENV_VAR).as_deref() {
        Ok("on") => {
            let stdout_json_layer = tracing_subscriber::fmt::layer().json().flatten_event(true);
            tracing_subscriber::registry()
                .with(forensics_layer)
                .with(stdout_json_layer.with_filter(env_filter))
                .try_init()
                .map_err(wrap_tracing_error)?;
        }
        Ok("off") | Err(env::VarError::NotPresent) => {
            let stdout_compact_layer = tracing_subscriber::fmt::layer().compact();
            tracing_subscriber::registry()
                .with(forensics_layer)
                .with(stdout_compact_layer.with_filter(env_filter))
                .try_init()
                .map_err(wrap_tracing_error)?;
        }
        v => {
            bail!("invalid value for {LOG_JSON_ENV_VAR}: {:?}", v);
        }
    }

    Ok(())
}
