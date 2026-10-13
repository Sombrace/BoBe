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
    let mut secret = [0_u8; 32];
    rand::RngExt::fill(&mut rand::rng(), &mut secret);
    let token = format!("bobe_{}", crate::util::text::hex_encode(&secret));
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
}
