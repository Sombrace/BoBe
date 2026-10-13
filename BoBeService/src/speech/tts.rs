use crate::error::AppError;

pub(crate) trait TtsEngine: Send + Sync {
    fn synthesize(&self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>, AppError>;

    fn sample_rate(&self) -> u32;
}
