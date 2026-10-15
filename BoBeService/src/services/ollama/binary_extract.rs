use std::io::Read;
use std::path::{Component, Path};

use tracing::info;

use super::binary_download::{ArchiveFormat, OllamaArtifact};
use crate::error::AppError;

pub(crate) fn install_ollama_runtime(
    archive_path: &Path,
    install_dir: &Path,
    artifact: &OllamaArtifact,
) -> Result<(), AppError> {
    let invalid = || AppError::Config("Invalid Ollama runtime path".into());
    let parent = install_dir.parent().ok_or_else(invalid)?;
    let name = install_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(invalid)?;
    let staging = parent.join(format!(".{name}.partial"));
    remove_dir_if_present(&staging)?;
    std::fs::create_dir_all(&staging)?;

    let unpacked = unpack_archive(archive_path, &staging, artifact.format)
        .and_then(|()| verify_runtime(&staging, artifact));
    if let Err(error) = unpacked {
        let _ignored = std::fs::remove_dir_all(&staging);
        return Err(error);
    }

    remove_dir_if_present(install_dir)?;
    std::fs::rename(&staging, install_dir)?;
    info!(
        path = %install_dir.display(),
        version = artifact.version,
        "binary_extract.complete"
    );
    Ok(())
}

fn remove_dir_if_present(path: &Path) -> Result<(), AppError> {
    match std::fs::remove_dir_all(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

fn unpack_archive(archive_path: &Path, root: &Path, format: ArchiveFormat) -> Result<(), AppError> {
    let file = std::fs::File::open(archive_path)?;
    match format {
        ArchiveFormat::TarGz => unpack_tar(flate2::read::GzDecoder::new(file), root),
        ArchiveFormat::TarZstd => unpack_tar(zstd::stream::read::Decoder::new(file)?, root),
    }
}

fn unpack_tar(reader: impl Read, root: &Path) -> Result<(), AppError> {
    let mut archive = tar::Archive::new(reader);
    archive.set_unpack_xattrs(false);
    archive.set_preserve_permissions(false);
    archive.set_mask(0o022);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let entry_type = entry.header().entry_type();
        if entry_type == tar::EntryType::XGlobalHeader
            || (entry_type == tar::EntryType::Directory
                && path.components().all(|part| part == Component::CurDir))
        {
            continue;
        }
        if !is_relative_descendant(&path) {
            return Err(rejected(&path, "is outside the runtime directory"));
        }
        match entry_type {
            tar::EntryType::Regular | tar::EntryType::Directory => {}
            tar::EntryType::Symlink => {
                let target = entry
                    .link_name()?
                    .ok_or_else(|| rejected(&path, "has no link target"))?;
                if !is_relative_descendant(&target) {
                    return Err(rejected(&path, "links outside its directory"));
                }
            }
            other => {
                return Err(rejected(&path, &format!("has unsupported type {other:?}")));
            }
        }
        if !entry.unpack_in(root)? {
            return Err(rejected(&path, "could not be unpacked"));
        }
    }
    Ok(())
}

fn is_relative_descendant(path: &Path) -> bool {
    let mut named = false;
    for part in path.components() {
        match part {
            Component::Normal(_) => named = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    named
}

fn rejected(path: &Path, reason: &str) -> AppError {
    AppError::Config(format!("Ollama archive entry {} {reason}", path.display()))
}

fn verify_runtime(root: &Path, artifact: &OllamaArtifact) -> Result<(), AppError> {
    for relative in [artifact.binary, artifact.llama_server] {
        let metadata = std::fs::symlink_metadata(root.join(relative))
            .map_err(|_| AppError::Config(format!("Ollama archive is missing {relative}")))?;
        if !metadata.is_file() {
            return Err(AppError::Config(format!(
                "Ollama archive entry {relative} is not a file"
            )));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(AppError::Config(format!(
                    "Ollama archive entry {relative} is not executable"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::io::Write;
    use std::path::{Path, PathBuf};

    use super::*;

    enum Item {
        File(&'static str, u32, &'static [u8]),
        Dir(&'static str),
        Link(&'static str, &'static str),
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bobe-ollama-extract-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    fn artifact(
        format: ArchiveFormat,
        binary: &'static str,
        llama_server: &'static str,
    ) -> OllamaArtifact {
        OllamaArtifact {
            version: "0.0.0-test",
            url: "",
            sha256: "",
            max_bytes: 0,
            archive_name: "ollama-test",
            format,
            binary,
            llama_server,
        }
    }

    fn set_raw(field: &mut [u8], value: &str) {
        field[..value.len()].copy_from_slice(value.as_bytes());
    }

    fn tar_bytes(items: &[Item]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for item in items {
            let mut header = tar::Header::new_gnu();
            let data: &[u8] = match item {
                Item::File(path, mode, data) => {
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_mode(*mode);
                    set_raw(&mut header.as_old_mut().name, path);
                    data
                }
                Item::Dir(path) => {
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_mode(0o755);
                    set_raw(&mut header.as_old_mut().name, path);
                    &[]
                }
                Item::Link(path, target) => {
                    header.set_entry_type(tar::EntryType::Symlink);
                    header.set_mode(0o777);
                    set_raw(&mut header.as_old_mut().name, path);
                    set_raw(&mut header.as_old_mut().linkname, target);
                    &[]
                }
            };
            header.set_size(data.len() as u64);
            header.set_cksum();
            builder.append(&header, data).expect("entry should append");
        }
        builder.into_inner().expect("archive should finish")
    }

    fn write_archive(dir: &Path, format: ArchiveFormat, items: &[Item]) -> PathBuf {
        let tar = tar_bytes(items);
        let compressed = match format {
            ArchiveFormat::TarGz => {
                let mut encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(&tar).expect("gzip should encode");
                encoder.finish().expect("gzip should finish")
            }
            ArchiveFormat::TarZstd => {
                zstd::stream::encode_all(&tar[..], 0).expect("zstd should encode")
            }
        };
        let path = dir.join("ollama-archive");
        std::fs::write(&path, compressed).expect("archive should be written");
        path
    }

    fn assert_executable(path: &Path) {
        let metadata = std::fs::symlink_metadata(path).expect("file should exist");
        assert!(metadata.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(metadata.permissions().mode() & 0o777, 0o755);
        }
    }

    #[test]
    fn installs_whole_flat_runtime_with_sibling_links() {
        let dir = temp_dir("flat");
        let archive = write_archive(
            &dir,
            ArchiveFormat::TarGz,
            &[
                Item::File("ollama", 0o755, b"binary"),
                Item::File("llama-server", 0o755, b"server"),
                Item::File("libggml.0.26.0.dylib", 0o644, b"library"),
                Item::Link("libggml.0.dylib", "libggml.0.26.0.dylib"),
                Item::Dir("mlx_metal_v3/"),
                Item::File("mlx_metal_v3/libmlx.dylib", 0o666, b"mlx"),
            ],
        );
        let install_dir = dir.join("ollama").join("v0.0.0-test");

        install_ollama_runtime(
            &archive,
            &install_dir,
            &artifact(ArchiveFormat::TarGz, "ollama", "llama-server"),
        )
        .expect("runtime should install");

        assert_executable(&install_dir.join("ollama"));
        assert_executable(&install_dir.join("llama-server"));
        assert_eq!(
            std::fs::read_link(install_dir.join("libggml.0.dylib")).expect("link should exist"),
            PathBuf::from("libggml.0.26.0.dylib")
        );
        assert_eq!(
            std::fs::read(install_dir.join("libggml.0.dylib")).expect("link should resolve"),
            b"library"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(install_dir.join("mlx_metal_v3/libmlx.dylib"))
                .expect("nested file should exist")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o644);
        }
        assert!(!dir.join("ollama").join(".v0.0.0-test.partial").exists());
        let _ignored = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn installs_nested_runtime_from_zstd() {
        let dir = temp_dir("nested");
        let archive = write_archive(
            &dir,
            ArchiveFormat::TarZstd,
            &[
                Item::File("bin/ollama", 0o755, b"binary"),
                Item::File("lib/ollama/llama-server", 0o755, b"server"),
                Item::File("lib/ollama/libggml.so.0.26.0", 0o644, b"library"),
                Item::Link("lib/ollama/libggml.so", "libggml.so.0.26.0"),
            ],
        );
        let install_dir = dir.join("v0.0.0-test");

        install_ollama_runtime(
            &archive,
            &install_dir,
            &artifact(
                ArchiveFormat::TarZstd,
                "bin/ollama",
                "lib/ollama/llama-server",
            ),
        )
        .expect("runtime should install");

        assert_executable(&install_dir.join("bin/ollama"));
        assert_executable(&install_dir.join("lib/ollama/llama-server"));
        assert_eq!(
            std::fs::read(install_dir.join("lib/ollama/libggml.so")).expect("link should resolve"),
            b"library"
        );
        let _ignored = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replaces_an_earlier_install_of_the_same_version() {
        let dir = temp_dir("replace");
        let install_dir = dir.join("v0.0.0-test");
        std::fs::create_dir_all(&install_dir).expect("old install should exist");
        std::fs::write(install_dir.join("stale"), b"old").expect("stale file should exist");
        let archive = write_archive(
            &dir,
            ArchiveFormat::TarGz,
            &[
                Item::File("ollama", 0o755, b"binary"),
                Item::File("llama-server", 0o755, b"server"),
            ],
        );

        install_ollama_runtime(
            &archive,
            &install_dir,
            &artifact(ArchiveFormat::TarGz, "ollama", "llama-server"),
        )
        .expect("runtime should install");

        assert!(!install_dir.join("stale").exists());
        assert_executable(&install_dir.join("ollama"));
        let _ignored = std::fs::remove_dir_all(dir);
    }

    fn assert_rejected(name: &str, items: &[Item], expected: &str) {
        let dir = temp_dir(name);
        let archive = write_archive(&dir, ArchiveFormat::TarGz, items);
        let install_dir = dir.join("runtime").join("v0.0.0-test");

        let error = install_ollama_runtime(
            &archive,
            &install_dir,
            &artifact(ArchiveFormat::TarGz, "ollama", "llama-server"),
        )
        .expect_err("archive should be rejected");

        assert!(
            error.to_string().contains(expected),
            "unexpected error: {error}"
        );
        assert!(!install_dir.exists());
        assert!(!dir.join("runtime").join(".v0.0.0-test.partial").exists());
        assert!(!dir.join("escape").exists());
        let _ignored = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_entries_outside_the_runtime() {
        assert_rejected(
            "parent-entry",
            &[
                Item::File("ollama", 0o755, b"binary"),
                Item::File("llama-server", 0o755, b"server"),
                Item::File("../../escape", 0o644, b"outside"),
            ],
            "is outside the runtime directory",
        );
        assert_rejected(
            "absolute-entry",
            &[Item::File("/tmp/escape", 0o644, b"outside")],
            "is outside the runtime directory",
        );
    }

    #[test]
    fn rejects_links_outside_the_runtime() {
        assert_rejected(
            "parent-link",
            &[
                Item::File("ollama", 0o755, b"binary"),
                Item::File("llama-server", 0o755, b"server"),
                Item::Link("libggml.dylib", "../../escape"),
            ],
            "links outside its directory",
        );
        assert_rejected(
            "absolute-link",
            &[Item::Link("libggml.dylib", "/etc/hosts")],
            "links outside its directory",
        );
    }

    #[test]
    fn rejects_a_runtime_without_llama_server() {
        assert_rejected(
            "missing-server",
            &[Item::File("ollama", 0o755, b"binary")],
            "missing llama-server",
        );
    }

    #[test]
    fn rejects_a_runtime_whose_binary_is_not_executable() {
        assert_rejected(
            "non-executable",
            &[
                Item::File("ollama", 0o644, b"binary"),
                Item::File("llama-server", 0o755, b"server"),
            ],
            "ollama is not executable",
        );
    }
}
