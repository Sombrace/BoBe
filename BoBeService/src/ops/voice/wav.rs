use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::AppError;

pub(super) struct WavOutput {
    file: File,
    path: PathBuf,
    samples: u32,
    finished: bool,
}

impl WavOutput {
    pub(super) fn create(path: &Path) -> Result<Self, AppError> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut output = Self {
            file,
            path: path.to_owned(),
            samples: 0,
            finished: false,
        };
        output.file.write_all(&[0; 44])?;
        Ok(output)
    }

    pub(super) fn append(&mut self, pcm: &[f32]) -> Result<(), AppError> {
        if self.finished || self.samples as usize + pcm.len() > 24_000 * 600 {
            return Err(AppError::Validation(
                "Voice preview exceeds the ten-minute audio limit".into(),
            ));
        }
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for sample in pcm {
            if !sample.is_finite() {
                return Err(AppError::Internal(
                    "Speech contains non-finite samples".into(),
                ));
            }
            bytes.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
        }
        self.file.write_all(&bytes)?;
        self.samples += pcm.len() as u32;
        Ok(())
    }

    pub(super) fn finish(&mut self) -> Result<(), AppError> {
        if self.finished {
            return Ok(());
        }
        let size = self.samples * 2;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(b"RIFF")?;
        self.file.write_all(&(36 + size).to_le_bytes())?;
        self.file.write_all(b"WAVEfmt ")?;
        self.file.write_all(&16_u32.to_le_bytes())?;
        self.file.write_all(&1_u16.to_le_bytes())?;
        self.file.write_all(&1_u16.to_le_bytes())?;
        self.file.write_all(&24_000_u32.to_le_bytes())?;
        self.file.write_all(&48_000_u32.to_le_bytes())?;
        self.file.write_all(&2_u16.to_le_bytes())?;
        self.file.write_all(&16_u16.to_le_bytes())?;
        self.file.write_all(b"data")?;
        self.file.write_all(&size.to_le_bytes())?;
        self.file.flush()?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for WavOutput {
    fn drop(&mut self) {
        if !self.finished {
            drop(std::fs::remove_file(&self.path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_never_overwrites_an_existing_file() {
        let path = std::env::temp_dir().join(format!("bobe-preview-{}.wav", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"owner audio").unwrap();
        assert!(WavOutput::create(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"owner audio");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn interrupted_export_has_a_valid_length_and_failed_exports_are_removed() {
        let path = std::env::temp_dir().join(format!("bobe-preview-{}.wav", uuid::Uuid::new_v4()));
        {
            let mut output = WavOutput::create(&path).unwrap();
            output.append(&[0.5; 480]).unwrap();
            output.finish().unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 960);
        assert_eq!(bytes.len(), 1_004);
        std::fs::remove_file(&path).unwrap();
        {
            let mut output = WavOutput::create(&path).unwrap();
            assert!(output.append(&[f32::NAN]).is_err());
        }
        assert!(!path.exists());
    }
}
