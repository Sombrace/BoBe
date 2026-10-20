use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{Builder, InitError, RollingFileAppender, Rotation};
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Config;

pub(crate) const DAILY_LOG_SUFFIX: &str = "log";

#[must_use = "dropping the guard stops the log file writer"]
pub(crate) fn init_tracing(config: &Config) -> Option<WorkerGuard> {
    let (filter, invalid_level) = match EnvFilter::try_new(&config.logging.level) {
        Ok(filter) => (filter, None),
        Err(error) => (EnvFilter::new("info"), Some(error)),
    };
    let (writer, guard, ansi) = match build_file_writer(&config.logging) {
        Some((writer, guard)) => (BoxMakeWriter::new(writer), Some(guard), false),
        None => (
            BoxMakeWriter::new(std::io::stdout),
            None,
            std::io::stdout().is_terminal(),
        ),
    };
    let layer = fmt::layer().with_writer(writer).with_ansi(ansi);
    let layer = if config.logging.json {
        layer.json().boxed()
    } else {
        layer.boxed()
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .init();
    if let Some(error) = invalid_level {
        tracing::warn!(%error, level = %config.logging.level, "logging.invalid_level");
    }
    guard
}

#[expect(clippy::print_stderr, reason = "logging is not initialized yet")]
fn build_file_writer(config: &crate::config::LoggingConfig) -> Option<(NonBlocking, WorkerGuard)> {
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
    Some(tracing_appender::non_blocking(appender))
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
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    Some((directory, file_name))
}
