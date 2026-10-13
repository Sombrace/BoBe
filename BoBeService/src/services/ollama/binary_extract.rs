use std::path::Path;

use tracing::info;

use super::binary_download::ArchiveFormat;
use crate::error::AppError;

pub(crate) fn extract_ollama_archive(
    archive_path: &Path,
    output_path: &Path,
    format: ArchiveFormat,
) -> Result<(), AppError> {
    match format {
        ArchiveFormat::TarGz => {
            let file = std::fs::File::open(archive_path)?;
            extract_from_tar(flate2::read::GzDecoder::new(file), output_path)
        }
        ArchiveFormat::TarZstd => {
            let file = std::fs::File::open(archive_path)?;
            let decoder = zstd::stream::read::Decoder::new(file)?;
            extract_from_tar(decoder, output_path)
        }
    }
}

fn extract_from_tar(decoder: impl std::io::Read, output_path: &Path) -> Result<(), AppError> {
    let mut archive = tar::Archive::new(decoder);

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut found = false;
    let entries = archive.entries()?;

    for entry in entries {
        let mut entry = entry?;
        let entry_path = entry.path()?.into_owned();

        if entry_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            tracing::warn!(path = %entry_path.display(), "binary_extract.path_traversal_blocked");
            continue;
        }

        let file_name = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if file_name == "ollama" && !entry.header().entry_type().is_dir() {
            info!(
                entry = %entry_path.display(),
                target = %output_path.display(),
                "binary_extract.extracting_binary"
            );

            let mut output_file = std::fs::File::create(output_path)?;
            std::io::copy(&mut entry, &mut output_file)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(0o755);
                std::fs::set_permissions(output_path, perms)?;
            }

            found = true;
            break;
        }
    }

    if !found {
        return Err(AppError::Config(
            "Ollama binary not found in archive".into(),
        ));
    }

    info!(
        path = %output_path.display(),
        "binary_extract.complete"
    );

    Ok(())
}
