use std::path::{Path, PathBuf};

use crate::error::AppError;

pub(super) fn tempfile_dir(base: &Path) -> Result<PathBuf, std::io::Error> {
    std::fs::create_dir_all(base)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(base, std::fs::Permissions::from_mode(0o700))?;
    }
    let unique = format!(
        ".voice-install-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    );
    let path = base.join(unique);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir(&path)?;
    Ok(path)
}

pub(super) async fn extract_and_install(
    archive: &Path,
    tmp_dir: &Path,
    final_target: &Path,
) -> Result<(), AppError> {
    let archive_owned = archive.to_path_buf();
    let tmp_owned = tmp_dir.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let file = std::fs::File::open(&archive_owned)?;
        let bz = bzip2::read::BzDecoder::new(file);
        let mut tar = tar::Archive::new(bz);
        tar.unpack(&tmp_owned)?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("untar join: {e}")))??;

    let mut extracted: Option<PathBuf> = None;
    let mut entries = tokio::fs::read_dir(tmp_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            extracted = Some(entry.path());
            break;
        }
    }
    let extracted =
        extracted.ok_or_else(|| AppError::Internal("tarball had no directory inside".into()))?;

    install_directory(&extracted, final_target).await?;
    if let Err(error) = tokio::fs::remove_file(archive).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %archive.display(), error = %error, "voice_install.archive_cleanup_failed");
    }
    Ok(())
}

pub(super) async fn install_directory(
    extracted: &Path,
    final_target: &Path,
) -> Result<(), AppError> {
    if let Some(parent) = final_target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let backup = final_target.with_extension("replaced");
    if tokio::fs::try_exists(&backup).await? {
        tokio::fs::remove_dir_all(&backup).await?;
    }
    let had_existing = tokio::fs::try_exists(final_target).await?;
    if had_existing {
        tokio::fs::rename(final_target, &backup).await?;
    }
    if let Err(error) = tokio::fs::rename(extracted, final_target).await {
        if had_existing {
            drop(tokio::fs::rename(&backup, final_target).await);
        }
        return Err(error.into());
    }
    if had_existing {
        tokio::fs::remove_dir_all(&backup).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_directory_is_private_and_inside_models_filesystem() {
        let root = std::env::temp_dir().join(format!("bobe-voice-stage-{}", uuid::Uuid::new_v4()));
        let staging = tempfile_dir(&root).unwrap();

        assert_eq!(staging.parent(), Some(root.as_path()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            assert_eq!(
                std::fs::metadata(&root).unwrap().dev(),
                std::fs::metadata(&staging).unwrap().dev()
            );
            assert_eq!(
                std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&staging).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
