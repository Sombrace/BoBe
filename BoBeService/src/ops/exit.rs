use serde::Serialize;

use crate::error::AppError;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[repr(u8)]
pub(crate) enum ExitCode {
    Success = 0,
    Failed = 1,
    Invalid = 2,
    NotReady = 3,
    AuthorityConflict = 4,
}

impl ExitCode {
    pub(crate) fn process(self) -> std::process::ExitCode {
        std::process::ExitCode::from(self as u8)
    }
}

pub(crate) fn app_error_exit(error: &AppError) -> ExitCode {
    match error {
        AppError::Config(_) | AppError::Validation(_) | AppError::NotNetworked(_) => {
            ExitCode::Invalid
        }
        AppError::Conflict(_) | AppError::ProtocolIncompatible(_) => ExitCode::AuthorityConflict,
        AppError::Database(_)
        | AppError::RequestReplayUnsafe(_)
        | AppError::Unauthorized(_)
        | AppError::NotFound(_)
        | AppError::Capture(_)
        | AppError::ServiceUnavailable(_)
        | AppError::Canceled(_)
        | AppError::Serialization(_)
        | AppError::Io(_)
        | AppError::Network(_)
        | AppError::Internal(_) => ExitCode::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_errors_map_to_stable_operator_exit_codes() {
        assert_eq!(
            app_error_exit(&AppError::Config("bad".into())),
            ExitCode::Invalid
        );
        assert_eq!(
            app_error_exit(&AppError::NotNetworked("loopback".into())),
            ExitCode::Invalid
        );
        assert_eq!(
            app_error_exit(&AppError::Conflict("owned".into())),
            ExitCode::AuthorityConflict
        );
        assert_eq!(
            app_error_exit(&AppError::Internal("failed".into())),
            ExitCode::Failed
        );
    }
}
