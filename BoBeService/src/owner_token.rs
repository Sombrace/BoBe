use crate::error::AppError;

pub(crate) fn read(paths: &crate::util::paths::DataPaths) -> Result<Option<String>, AppError> {
    let path = paths.owner_api_token_file();
    match std::fs::read_to_string(path) {
        Ok(value) => {
            let token = value.trim().to_owned();
            validate(&token)?;
            Ok(Some(token))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::Io(error)),
    }
}

pub(crate) fn ensure(paths: &crate::util::paths::DataPaths) -> Result<String, AppError> {
    if let Some(token) = read(paths)? {
        return Ok(token);
    }
    let mut secret = zeroize::Zeroizing::new([0_u8; 32]);
    rand::fill(secret.as_mut_slice());
    let token = format!("bobe_{}", crate::util::text::hex_encode(secret.as_slice()));
    let path = paths.owner_api_token_file();
    #[cfg(unix)]
    crate::util::durable_fs::atomic_write_sync_mode(&path, token.as_bytes(), 0o600)?;
    #[cfg(not(unix))]
    crate::util::durable_fs::atomic_write_sync(&path, token.as_bytes())?;
    Ok(token)
}

fn validate(token: &str) -> Result<(), AppError> {
    if token.is_empty()
        || token.len() > 1024
        || token
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(AppError::Config(
            "owner API token must be one non-empty token without whitespace or control characters"
                .into(),
        ));
    }
    if token.starts_with(crate::api::middleware::ENDPOINT_CREDENTIAL_PREFIX) {
        return Err(AppError::Config(format!(
            "owner API token must not start with {}, which is reserved for paired device credentials",
            crate::api::middleware::ENDPOINT_CREDENTIAL_PREFIX
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_persists_one_private_token() {
        let root = std::env::temp_dir().join(format!("bobe-owner-token-{}", uuid::Uuid::new_v4()));
        let paths = crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}", root.join("data.sqlite3").display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        let first = ensure(&paths).unwrap();
        let second = ensure(&paths).unwrap();
        assert_eq!(first, second);
        assert_eq!(read(&paths).unwrap().as_deref(), Some(first.as_str()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(paths.owner_api_token_file())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn read_accepts_an_operator_supplied_token() {
        let root =
            std::env::temp_dir().join(format!("bobe-supplied-token-{}", uuid::Uuid::new_v4()));
        let paths = crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}", root.join("data.sqlite3").display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        std::fs::write(paths.owner_api_token_file(), "operator-owner-token").unwrap();

        assert_eq!(
            read(&paths).unwrap().as_deref(),
            Some("operator-owner-token")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_owner_token_cannot_look_like_a_paired_device_credential() {
        let root = std::env::temp_dir().join(format!("bobe-v2-token-{}", uuid::Uuid::new_v4()));
        let paths = crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}", root.join("data.sqlite3").display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        std::fs::write(paths.owner_api_token_file(), "bobe_v2_operator-owner-token").unwrap();

        let error = read(&paths).unwrap_err().to_string();

        assert!(
            error.contains("reserved for paired device credentials"),
            "{error}"
        );
        assert!(validate("bobe_operator-owner-token").is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
}
