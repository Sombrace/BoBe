use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{Builder, InitError, RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Config;

static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

pub(crate) const DAILY_LOG_SUFFIX: &str = "log";

pub(crate) fn init_tracing(config: &Config) {
    let filter =
        EnvFilter::try_new(&config.logging.level).unwrap_or_else(|_| EnvFilter::new("info"));
    let file_writer = build_file_writer(&config.logging);

    if config.logging.json {
        if let Some(writer) = file_writer {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json().with_writer(writer))
                .init();
        } else {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json())
                .init();
        }
    } else if let Some(writer) = file_writer {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().with_target(true).with_writer(writer))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().with_target(true))
            .init();
    }
}

#[allow(clippy::print_stderr)]
fn build_file_writer(config: &crate::config::LoggingConfig) -> Option<NonBlocking> {
    let (dir, file_name) = daily_log_target(config)?;

    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!(
            "Failed to create log directory '{}': {err}; using stdout logging.",
            dir.display()
        );
        return None;
    }

    let appender = match daily_appender(&dir, &file_name) {
        Ok(appender) => appender,
        Err(err) => {
            eprintln!(
                "Failed to build rolling log appender at '{}': {err}; using stdout logging.",
                dir.display()
            );
            return None;
        }
    };
    let (writer, guard) = tracing_appender::non_blocking(appender);
    if LOG_GUARD.set(guard).is_err() {
        eprintln!("Tracing worker guard was already initialized.");
    }
    Some(writer)
}

pub(crate) fn daily_appender(
    directory: &Path,
    file_name: &str,
) -> Result<RollingFileAppender, InitError> {
    Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(file_name)
        .filename_suffix(DAILY_LOG_SUFFIX)
        .latest_symlink(file_name)
        .build(directory)
}

pub(crate) fn daily_log_target(config: &crate::config::LoggingConfig) -> Option<(PathBuf, String)> {
    let path = config.file.as_deref()?.trim();
    if path.is_empty() {
        return None;
    }

    let file_path = crate::util::paths::expand_tilde(path);
    let file_name = file_path.file_name()?.to_str()?.to_owned();
    if file_name.is_empty() {
        return None;
    }
    let directory = file_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), std::path::Path::to_path_buf);
    Some((directory, file_name))
}
