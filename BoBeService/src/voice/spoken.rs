use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::provider::InterruptedReply;

struct SpokenSentence {
    text: String,
    end_chunk: Option<u64>,
}

#[derive(Default)]
pub(crate) struct SpokenScript {
    sentences: Mutex<Vec<SpokenSentence>>,
    finished: AtomicBool,
    audio_ms: AtomicU64,
}

impl SpokenScript {
    pub(crate) fn push(&self, text: &str, end_chunk: u64) {
        self.sentences
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SpokenSentence {
                text: text.trim().to_owned(),
                end_chunk: Some(end_chunk),
            });
    }

    pub(crate) fn begin_sentence(&self, text: &str) -> usize {
        let mut sentences = self
            .sentences
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let index = sentences.len();
        sentences.push(SpokenSentence {
            text: text.trim().to_owned(),
            end_chunk: None,
        });
        index
    }

    pub(crate) fn complete_sentence(&self, index: usize, end_chunk: u64) {
        if let Some(sentence) = self
            .sentences
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(index)
        {
            sentence.end_chunk = Some(end_chunk);
        }
    }

    pub(crate) fn add_audio(&self, duration: Duration) {
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        self.audio_ms.fetch_add(millis, Ordering::Relaxed);
    }

    pub(crate) fn audio_duration(&self) -> Option<Duration> {
        match self.audio_ms.load(Ordering::Relaxed) {
            0 => None,
            millis => Some(Duration::from_millis(millis)),
        }
    }

    pub(crate) fn finish(&self) {
        self.finished.store(true, Ordering::Release);
    }

    pub(crate) fn heard_until(&self, playhead_chunk: u64) -> Option<InterruptedReply> {
        let sentences = self
            .sentences
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let heard_count = sentences
            .iter()
            .take_while(|sentence| sentence.end_chunk.is_some_and(|end| end <= playhead_chunk))
            .count();
        let cut_off = sentences
            .get(heard_count)
            .map(|sentence| sentence.text.clone());
        if cut_off.is_none() && self.finished.load(Ordering::Acquire) {
            return None;
        }
        let heard = sentences
            .iter()
            .take(heard_count)
            .map(|sentence| sentence.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        Some(InterruptedReply { heard, cut_off })
    }
}

#[cfg(test)]
mod tests {
    use super::SpokenScript;

    fn script(ends: &[(&str, u64)]) -> SpokenScript {
        let script = SpokenScript::default();
        for (text, end_chunk) in ends {
            script.push(text, *end_chunk);
        }
        script
    }

    #[test]
    fn sentences_played_in_full_are_heard_and_the_next_is_cut_off() {
        let script = script(&[("One.", 10), ("Two.", 25), ("Three.", 40)]);
        script.finish();
        let reply = script.heard_until(25).expect("interrupted");
        assert_eq!(reply.heard, "One. Two.");
        assert_eq!(reply.cut_off.as_deref(), Some("Three."));

        let inside = script.heard_until(24).expect("interrupted");
        assert_eq!(inside.heard, "One.");
        assert_eq!(inside.cut_off.as_deref(), Some("Two."));
    }

    #[test]
    fn nothing_is_heard_before_the_first_sentence_ends() {
        let script = script(&[("One.", 10), ("Two.", 25)]);
        let reply = script.heard_until(0).expect("interrupted");
        assert_eq!(reply.heard, "");
        assert_eq!(reply.cut_off.as_deref(), Some("One."));
    }

    #[test]
    fn synthesised_audio_is_totalled_only_when_the_daemon_made_it() {
        let script = script(&[("One.", 1)]);
        assert!(script.audio_duration().is_none());
        script.add_audio(std::time::Duration::from_millis(1_500));
        script.add_audio(std::time::Duration::from_millis(2_500));
        assert_eq!(
            script.audio_duration(),
            Some(std::time::Duration::from_secs(4))
        );
    }

    #[test]
    fn a_reply_played_to_its_end_was_not_interrupted() {
        let script = script(&[("One.", 10), ("Two.", 25)]);
        script.finish();
        assert!(script.heard_until(25).is_none());
    }

    #[test]
    fn a_reply_still_being_written_is_interrupted_between_sentences() {
        let script = script(&[("One.", 1), ("Two.", 2)]);
        let reply = script.heard_until(2).expect("interrupted");
        assert_eq!(reply.heard, "One. Two.");
        assert!(reply.cut_off.is_none());
    }

    #[test]
    fn catching_up_with_generation_does_not_mark_an_unfinished_sentence_heard() {
        let script = script(&[("One.", 10)]);
        let pending = script.begin_sentence("Still being generated.");
        let reply = script.heard_until(25).expect("interrupted");
        assert_eq!(reply.heard, "One.");
        assert_eq!(reply.cut_off.as_deref(), Some("Still being generated."));
        script.complete_sentence(pending, 30);
        script.finish();
        assert!(script.heard_until(30).is_none());
    }
}
