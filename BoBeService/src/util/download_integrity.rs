use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::util::text::hex_encode;

pub(crate) struct DownloadIntegrity {
    label: &'static str,
    expected_sha256: &'static str,
    max_bytes: u64,
    received: u64,
    hasher: Sha256,
}

impl DownloadIntegrity {
    pub(crate) fn new(label: &'static str, expected_sha256: &'static str, max_bytes: u64) -> Self {
        Self {
            label,
            expected_sha256,
            max_bytes,
            received: 0,
            hasher: Sha256::new(),
        }
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) -> Result<(), AppError> {
        self.received = self
            .received
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| AppError::Config(format!("{} download size overflow", self.label)))?;
        if self.received > self.max_bytes {
            return Err(AppError::Config(format!(
                "{} download exceeded {} byte limit",
                self.label, self.max_bytes
            )));
        }
        self.hasher.update(bytes);
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<u64, AppError> {
        let actual = hex_encode(&self.hasher.finalize());
        if actual != self.expected_sha256 {
            return Err(AppError::Config(format!(
                "{} SHA-256 mismatch: expected {}, got {actual}",
                self.label, self.expected_sha256
            )));
        }
        Ok(self.received)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on fixture failures")]
mod tests {
    use super::*;

    #[test]
    fn accepts_matching_digest() {
        let mut integrity = DownloadIntegrity::new(
            "fixture",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            3,
        );
        integrity.update(b"abc").expect("within size limit");
        assert_eq!(integrity.finish().expect("digest matches"), 3);
    }

    #[test]
    fn rejects_oversized_and_mismatched_data() {
        let mut oversized = DownloadIntegrity::new("fixture", "", 2);
        assert!(oversized.update(b"abc").is_err());

        let mut mismatched = DownloadIntegrity::new("fixture", "deadbeef", 3);
        mismatched.update(b"abc").expect("within size limit");
        assert!(mismatched.finish().is_err());
    }
}
