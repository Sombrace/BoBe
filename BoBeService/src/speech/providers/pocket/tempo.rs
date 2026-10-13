/// Pitch-preserving streaming speed adjustment. Compensate for both sides of
/// the analysis window so faster speech neither clips the last word nor starts
/// with the time stretcher's silent pre-roll.
pub(super) struct Tempo {
    stretch: signalsmith_stretch::Stretch,
    speed: f64,
    input_samples: usize,
    processed_samples: usize,
    skip: usize,
    emitted: usize,
}

impl Tempo {
    pub(super) fn new(sample_rate: u32, speed: f32) -> Self {
        let stretch = signalsmith_stretch::Stretch::preset_default(1, sample_rate);
        let speed = f64::from(speed);
        let skip =
            (stretch.input_latency() as f64 / speed).round() as usize + stretch.output_latency();
        Self {
            stretch,
            speed,
            input_samples: 0,
            processed_samples: 0,
            skip,
            emitted: 0,
        }
    }

    pub(super) fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.input_samples += input.len();
        self.process_inner(input)
    }

    fn process_inner(&mut self, input: &[f32]) -> Vec<f32> {
        let before = (self.processed_samples as f64 / self.speed).round() as usize;
        self.processed_samples += input.len();
        let after = (self.processed_samples as f64 / self.speed).round() as usize;
        let mut output = vec![0.0; after - before];
        self.stretch.process(input, &mut output);
        self.trim(output)
    }

    fn trim(&mut self, mut output: Vec<f32>) -> Vec<f32> {
        let skipped = self.skip.min(output.len());
        output.drain(..skipped);
        self.skip -= skipped;
        let target = (self.input_samples as f64 / self.speed).round() as usize;
        output.truncate(target.saturating_sub(self.emitted));
        self.emitted += output.len();
        output
    }

    pub(super) fn finish(&mut self) -> Vec<f32> {
        let mut output = self.process_inner(&vec![0.0; self.stretch.input_latency()]);
        let mut tail = vec![0.0; self.stretch.output_latency()];
        self.stretch.flush(&mut tail);
        output.extend(self.trim(tail));
        output
    }
}

#[cfg(test)]
mod tests {
    use super::Tempo;

    #[test]
    fn tempo_preserves_pitch_and_duration_across_chunk_boundaries() {
        let sample_rate = 24_000;
        let signal: Vec<f32> = (0..sample_rate)
            .map(|n| (n as f32 * 440.0 * std::f32::consts::TAU / sample_rate as f32).sin() * 0.5)
            .collect();
        for speed in [0.5, 0.8, 1.5, 2.0] {
            let mut tempo = Tempo::new(sample_rate, speed);
            let mut output = Vec::new();
            for chunk in signal.chunks(719) {
                output.extend(tempo.process(chunk));
            }
            output.extend(tempo.finish());
            let expected = (sample_rate as f64 / f64::from(speed)).round() as usize;
            assert!(output.len().abs_diff(expected) <= 1);
            assert!(output.iter().all(|s| s.is_finite()));
            let middle = &output[output.len() / 4..output.len() * 3 / 4];
            let crossings = middle
                .windows(2)
                .filter(|s| s[0] < 0.0 && s[1] >= 0.0)
                .count();
            let frequency = crossings as f64 * f64::from(sample_rate) / middle.len() as f64;
            assert!(
                (frequency - 440.0).abs() < 12.0,
                "pitch changed at {speed}: {frequency}"
            );
        }
    }
}
