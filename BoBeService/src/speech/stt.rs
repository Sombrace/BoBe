use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SttDescriptor {
    pub(crate) id: &'static str,
    pub(crate) version: &'static str,
    pub(crate) languages: &'static [&'static str],
    pub(crate) sample_rate: u32,
    pub(crate) partials: bool,
    pub(crate) eou: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SttHealth {
    pub(crate) real_time_factor: f64,
    pub(crate) measured_audio: Duration,
}

impl SttHealth {
    pub(crate) const MAX_VIABLE_REAL_TIME_FACTOR: f64 = 0.5;

    pub(crate) fn is_viable(&self) -> bool {
        self.measured_audio >= Duration::from_secs(1)
            && self.real_time_factor.is_finite()
            && self.real_time_factor >= 0.0
            && self.real_time_factor < Self::MAX_VIABLE_REAL_TIME_FACTOR
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SttEvent {
    Partial(String),
    Final(String),
}

pub(crate) trait SttStream: Send {
    fn accept_pcm_s16le(&mut self, frame: &[u8]) -> Result<Vec<SttEvent>, String>;
    fn finish(&mut self) -> Result<Vec<SttEvent>, String>;
}

pub(crate) trait SttEngine: Send + Sync {
    fn descriptor(&self) -> &SttDescriptor;
    fn health(&self) -> &SttHealth;
    fn create_stream(&self, endpoint_finals: bool) -> Result<Box<dyn SttStream>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viability_requires_a_real_probe_below_threshold() {
        assert!(
            !SttHealth {
                real_time_factor: 0.1,
                measured_audio: Duration::from_millis(999),
            }
            .is_viable()
        );
        assert!(
            SttHealth {
                real_time_factor: 0.49,
                measured_audio: Duration::from_secs(1),
            }
            .is_viable()
        );
        assert!(
            !SttHealth {
                real_time_factor: 0.5,
                measured_audio: Duration::from_secs(1),
            }
            .is_viable()
        );
    }
}
