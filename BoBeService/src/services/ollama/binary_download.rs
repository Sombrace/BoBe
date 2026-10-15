use std::path::Path;
use std::time::Duration;

use futures::StreamExt;
use tokio::sync::watch;
use tracing::info;

use crate::error::AppError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArchiveFormat {
    TarGz,
    TarZstd,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct OllamaArtifact {
    pub(crate) version: &'static str,
    pub(crate) url: &'static str,
    pub(crate) sha256: &'static str,
    pub(crate) max_bytes: u64,
    pub(crate) archive_name: &'static str,
    pub(crate) format: ArchiveFormat,
    pub(crate) binary: &'static str,
    pub(crate) llama_server: &'static str,
}

pub(crate) const fn current_artifact() -> Result<OllamaArtifact, &'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64"))
        || cfg!(all(target_os = "macos", target_arch = "x86_64"))
    {
        return Ok(OllamaArtifact {
            version: "0.40.4",
            url: "https://github.com/ollama/ollama/releases/download/v0.40.4/ollama-darwin.tgz",
            sha256: "a4ceae4b2aaf18df37ebcbfb56ba3a37a87838631112d66c2be07268693c8d58",
            max_bytes: 200_000_000,
            archive_name: "ollama-darwin.tgz",
            format: ArchiveFormat::TarGz,
            binary: "ollama",
            llama_server: "llama-server",
        });
    }
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Ok(OllamaArtifact {
            version: "0.40.4",
            url: "https://github.com/ollama/ollama/releases/download/v0.40.4/ollama-linux-amd64.tar.zst",
            sha256: "15f31e9576d15a481e099b4145dba6e9f8c1f295927d6b8895300ae30ed3740a",
            max_bytes: 1_600_000_000,
            archive_name: "ollama-linux-amd64.tar.zst",
            format: ArchiveFormat::TarZstd,
            binary: "bin/ollama",
            llama_server: "lib/ollama/llama-server",
        });
    }
    Err("managed Ollama installation supports macOS and Linux x86_64 only")
}

const OLLAMA_DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(30);

pub(crate) async fn download_ollama(
    client: &reqwest::Client,
    artifact: &OllamaArtifact,
    output_path: &Path,
    cancel_rx: watch::Receiver<bool>,
    on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), AppError> {
    download_from_url(
        client,
        artifact.url,
        artifact.sha256,
        artifact.max_bytes,
        output_path,
        cancel_rx,
        on_progress,
    )
    .await
}

async fn download_from_url(
    client: &reqwest::Client,
    url: &str,
    expected_sha256: &'static str,
    max_bytes: u64,
    output_path: &Path,
    mut cancel_rx: watch::Receiver<bool>,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), AppError> {
    info!(url, "binary_download.starting");

    let request = client.get(url).timeout(OLLAMA_DOWNLOAD_TIMEOUT).send();
    tokio::pin!(request);
    let response = tokio::select! {
        biased;
        () = wait_for_cancel(&mut cancel_rx) => {
            return Err(AppError::Canceled("Ollama download canceled".into()));
        }
        result = &mut request => result?,
    };

    if !response.status().is_success() {
        return Err(AppError::Config(format!(
            "Ollama download failed with status {}",
            response.status()
        )));
    }

    let total_size = response.content_length();
    if total_size.is_some_and(|size| size > max_bytes) {
        return Err(AppError::Config(format!(
            "Ollama download Content-Length exceeds {max_bytes} byte limit"
        )));
    }
    info!(total_bytes = ?total_size, "binary_download.content_length");

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let partial_path = output_path.with_file_name(format!(
        "{}.part",
        output_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| AppError::Config("Invalid Ollama archive name".into()))?
    ));
    let _ignored = tokio::fs::remove_file(&partial_path).await;

    let mut file = tokio::fs::File::create(&partial_path).await?;

    let result = async {
        let mut stream = response.bytes_stream();
        let mut integrity = crate::util::download_integrity::DownloadIntegrity::new(
            "Ollama",
            expected_sha256,
            max_bytes,
        );
        let mut downloaded: u64 = 0;
        let mut last_progress: u64 = 0;

        loop {
            let chunk = tokio::select! {
                biased;
                () = wait_for_cancel(&mut cancel_rx) => {
                    return Err(AppError::Canceled("Ollama download canceled".into()));
                }
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk?;
            integrity.update(&chunk)?;
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
            downloaded = downloaded
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| AppError::Config("Ollama download size overflow".into()))?;

            if downloaded - last_progress > 1_000_000 {
                on_progress(downloaded, total_size);
                last_progress = downloaded;
            }
        }

        tokio::io::AsyncWriteExt::flush(&mut file).await?;
        let verified_bytes = integrity.finish()?;
        debug_assert_eq!(downloaded, verified_bytes);
        on_progress(downloaded, total_size);
        Ok::<_, AppError>(downloaded)
    }
    .await;

    let downloaded = match result {
        Ok(downloaded) => downloaded,
        Err(error) => {
            let _ignored = tokio::fs::remove_file(&partial_path).await;
            return Err(error);
        }
    };

    if output_path.exists() {
        std::fs::remove_file(output_path)?;
    }
    tokio::fs::rename(&partial_path, output_path).await?;

    info!(
        bytes = downloaded,
        path = %output_path.display(),
        "binary_download.complete"
    );

    Ok(())
}

async fn wait_for_cancel(cancel_rx: &mut watch::Receiver<bool>) {
    if *cancel_rx.borrow() {
        return;
    }
    while cancel_rx.changed().await.is_ok() {
        if *cancel_rx.borrow() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::convert::Infallible;
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Bytes;
    use axum::routing::get;
    use tokio::net::TcpListener;
    use tokio::sync::{Notify, watch};

    use super::*;

    #[test]
    fn current_platform_uses_a_supported_artifact() {
        let artifact = current_artifact().expect("developer platform should be supported");
        assert!(
            artifact
                .url
                .starts_with("https://github.com/ollama/ollama/releases/")
        );
        assert!(
            artifact
                .url
                .contains(&format!("/download/v{}/", artifact.version))
        );
        assert!(artifact.url.ends_with(artifact.archive_name));
        assert_eq!(artifact.sha256.len(), 64);
        assert!(artifact.max_bytes > 100_000_000);
        for relative in [artifact.binary, artifact.llama_server] {
            assert!(
                Path::new(relative)
                    .components()
                    .all(|component| matches!(component, std::path::Component::Normal(_)))
            );
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn linux_x86_64_uses_native_zstd_artifact() {
        let artifact = current_artifact().expect("Linux x86_64 is supported");
        assert_eq!(artifact.archive_name, "ollama-linux-amd64.tar.zst");
        assert_eq!(artifact.format, ArchiveFormat::TarZstd);
        assert_eq!(artifact.binary, "bin/ollama");
        assert_eq!(artifact.llama_server, "lib/ollama/llama-server");
    }

    #[tokio::test]
    async fn cancellation_removes_partial_download() {
        crate::util::tls::install_crypto_provider().expect("TLS provider should install");
        let first_chunk_sent = Arc::new(Notify::new());
        let release_stream = Arc::new(Notify::new());
        let app = Router::new().route(
            "/ollama.tgz",
            get({
                let first_chunk_sent = Arc::clone(&first_chunk_sent);
                let release_stream = Arc::clone(&release_stream);
                move || {
                    let first_chunk_sent = Arc::clone(&first_chunk_sent);
                    let release_stream = Arc::clone(&release_stream);
                    async move {
                        let stream = async_stream::stream! {
                            yield Ok::<_, Infallible>(Bytes::from_static(b"partial"));
                            first_chunk_sent.notify_one();
                            release_stream.notified().await;
                        };
                        axum::body::Body::from_stream(stream)
                    }
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let address = listener.local_addr().expect("test address should exist");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test server should run");
        });

        let dir =
            std::env::temp_dir().join(format!("bobe-download-cancel-{}", uuid::Uuid::new_v4()));
        let output_path = dir.join("ollama.tgz");
        let partial_path = dir.join("ollama.tgz.part");
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let client = reqwest::Client::new();
        let url = format!("http://{address}/ollama.tgz");
        let download = tokio::spawn({
            let output_path = output_path.clone();
            async move {
                download_from_url(
                    &client,
                    &url,
                    "00",
                    1_000_000,
                    &output_path,
                    cancel_rx,
                    |_, _| {},
                )
                .await
            }
        });

        first_chunk_sent.notified().await;
        cancel_tx.send(true).expect("cancel receiver should exist");
        let error = tokio::time::timeout(Duration::from_secs(1), download)
            .await
            .expect("cancellation should be prompt")
            .expect("download task should join")
            .expect_err("download should be canceled");

        std::assert_matches!(error, AppError::Canceled(_));
        assert!(!partial_path.exists());
        assert!(!output_path.exists());
        release_stream.notify_waiters();
        server.abort();
        drop(server.await);
        let _ignored = std::fs::remove_dir_all(dir);
    }
}
