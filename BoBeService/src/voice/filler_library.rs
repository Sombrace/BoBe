use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::{info, warn};

use crate::speech::TtsEngine;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FillerKind {
    Thinking,
    ToolWebSearch,
    ToolReadFile,
    ToolGeneric,
}

impl FillerKind {
    const fn phrases(self) -> &'static [&'static str] {
        match self {
            Self::Thinking => &[
                "Hmm, let me think.",
                "Let me think about that.",
                "Give me a second.",
            ],
            Self::ToolWebSearch => &["Let me search the web.", "On it, searching."],
            Self::ToolReadFile => &["One sec, looking at that.", "Let me read that."],
            Self::ToolGeneric => &["Looking into that.", "On it."],
        }
    }

    fn all() -> &'static [FillerKind] {
        &[
            Self::Thinking,
            Self::ToolWebSearch,
            Self::ToolReadFile,
            Self::ToolGeneric,
        ]
    }
}

struct FillerSet {
    clips: Vec<Arc<Vec<f32>>>,
    next: AtomicUsize,
}

impl FillerSet {
    fn take(&self) -> Option<Arc<Vec<f32>>> {
        if self.clips.is_empty() {
            return None;
        }
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.clips.len();
        self.clips.get(index).map(Arc::clone)
    }
}

pub(crate) struct FillerLibrary {
    inner: HashMap<FillerKind, FillerSet>,
    sample_rate: u32,
}

impl FillerLibrary {
    pub(crate) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub(crate) fn get(&self, kind: FillerKind) -> Option<Arc<Vec<f32>>> {
        self.inner.get(&kind).and_then(FillerSet::take)
    }

    pub(crate) async fn render(tts: Arc<dyn TtsEngine>) -> Self {
        const VOICE: &str = crate::constants::voice_wire::DEFAULT_PERSONA;
        let sample_rate = tts.sample_rate();
        let results = tokio::task::spawn_blocking(move || {
            FillerKind::all()
                .iter()
                .flat_map(|kind| kind.phrases().iter().map(move |phrase| (*kind, *phrase)))
                .map(|(kind, phrase)| (kind, tts.synthesize(phrase, VOICE, 1.0)))
                .collect::<Vec<_>>()
        })
        .await;
        let mut inner: HashMap<FillerKind, FillerSet> = HashMap::new();
        match results {
            Ok(results) => {
                for (kind, result) in results {
                    match result {
                        Ok(pcm) => {
                            info!(
                                kind = ?kind,
                                samples = pcm.len(),
                                "voice.filler_rendered"
                            );
                            inner
                                .entry(kind)
                                .or_insert_with(|| FillerSet {
                                    clips: Vec::new(),
                                    next: AtomicUsize::new(0),
                                })
                                .clips
                                .push(Arc::new(pcm));
                        }
                        Err(error) => {
                            warn!(kind = ?kind, %error, "voice.filler_synth_failed");
                        }
                    }
                }
            }
            Err(error) => {
                warn!(%error, "voice.filler_synth_join_failed");
            }
        }
        Self { inner, sample_rate }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_kinds_have_phrases() {
        for kind in FillerKind::all() {
            assert!(kind.phrases().len() >= 2);
            assert!(kind.phrases().iter().all(|phrase| !phrase.is_empty()));
        }
    }

    #[test]
    fn phrases_avoid_apologies() {
        for kind in FillerKind::all() {
            for phrase in kind.phrases() {
                let p = phrase.to_lowercase();
                assert!(
                    !p.starts_with("sorry, i can") && !p.starts_with("apologies"),
                    "Filler '{p}' violates the no-apology rule (kind {kind:?})"
                );
            }
        }
    }

    #[test]
    fn no_duplicate_phrases() {
        let mut seen = std::collections::HashSet::new();
        for kind in FillerKind::all() {
            for phrase in kind.phrases() {
                assert!(seen.insert(*phrase), "duplicate filler phrase: {phrase:?}");
            }
        }
    }

    #[test]
    fn the_same_kind_never_repeats_a_clip_back_to_back() {
        let set = FillerSet {
            clips: vec![
                Arc::new(vec![0.0; 1]),
                Arc::new(vec![0.0; 2]),
                Arc::new(vec![0.0; 3]),
            ],
            next: AtomicUsize::new(0),
        };
        let played = (0..7)
            .map(|_| set.take().unwrap().len())
            .collect::<Vec<_>>();
        assert!(played.windows(2).all(|pair| pair[0] != pair[1]));
        assert_eq!(&played[..4], &[1, 2, 3, 1]);
    }

    #[test]
    fn a_kind_without_rendered_clips_plays_nothing() {
        let set = FillerSet {
            clips: Vec::new(),
            next: AtomicUsize::new(0),
        };
        assert!(set.take().is_none());
    }
}
