use crate::error::AppError;

pub(crate) trait TtsEngine: Send + Sync {
    fn synthesize(&self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>, AppError> {
        let mut audio = Vec::new();
        self.synthesize_stream(text, voice, speed, &mut |samples| {
            audio.extend_from_slice(samples);
            true
        })?;
        Ok(audio)
    }

    /// Runs on a blocking worker. Returning false from `emit` cancels generation.
    fn synthesize_stream(
        &self,
        text: &str,
        voice: &str,
        speed: f32,
        emit: &mut dyn FnMut(&[f32]) -> bool,
    ) -> Result<(), AppError>;

    fn sample_rate(&self) -> u32;
}
