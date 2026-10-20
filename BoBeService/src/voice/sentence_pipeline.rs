use crate::voice::text_prep::markdown_strip::{FenceFilter, strip_markdown};
use crate::voice::text_prep::sentence_buffer::{Sentence, SentenceBuffer};

pub(crate) struct SentencePipeline {
    fences: FenceFilter,
    sb: SentenceBuffer,
}

impl SentencePipeline {
    pub(crate) fn new() -> Self {
        Self {
            fences: FenceFilter::default(),
            sb: SentenceBuffer::new(),
        }
    }

    pub(crate) fn feed(&mut self, delta: &str) -> Vec<String> {
        let visible = self.fences.feed(delta);
        if visible.is_empty() {
            return Vec::new();
        }
        speakable(self.sb.feed(&visible))
    }

    pub(crate) fn settle(&mut self) -> Vec<String> {
        speakable(self.sb.settle())
    }

    pub(crate) fn flush(&mut self) -> Vec<String> {
        let tail = self.fences.flush();
        let mut sentences = self.sb.feed(&tail);
        sentences.extend(self.sb.flush());
        speakable(sentences)
    }
}

fn speakable(sentences: Vec<Sentence>) -> Vec<String> {
    sentences
        .into_iter()
        .map(|sentence| {
            strip_markdown(&sentence.text, sentence.starts_line)
                .trim()
                .to_owned()
        })
        .filter(|sentence| sentence.chars().any(char::is_alphanumeric))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spoken(deltas: &[&str]) -> Vec<String> {
        let mut pipeline = SentencePipeline::new();
        let mut sentences = Vec::new();
        for delta in deltas {
            sentences.extend(pipeline.feed(delta));
        }
        sentences.extend(pipeline.flush());
        sentences
    }

    #[test]
    fn markdown_is_removed_from_whole_sentences() {
        assert_eq!(
            spoken(&["**Hello.** Next sentence!"]),
            vec!["Hello.", "Next sentence!"]
        );
    }

    #[test]
    fn a_numbered_reply_streamed_token_by_token_loses_no_letters() {
        assert_eq!(
            spoken(&[
                "1",
                ".",
                " The",
                " Moon",
                " is",
                " close",
                ".",
                "\n",
                "2",
                ".",
                " Moon",
                "light",
                " is",
                " reflected",
                " sunlight",
                "."
            ]),
            vec!["The Moon is close.", "Moonlight is reflected sunlight."]
        );
    }

    #[test]
    fn emphasis_closed_in_a_later_delta_is_removed() {
        assert_eq!(
            spoken(&["This is **very", " important** today.", " Go."]),
            vec!["This is very important today.", "Go."]
        );
    }

    #[test]
    fn fenced_code_is_never_spoken() {
        assert_eq!(
            spoken(&["Try this:\n``", "`sh\nrm -rf x\n```\nThen", " check it."]),
            vec!["Try this:\n\nThen check it."]
        );
    }

    #[test]
    fn a_number_that_is_the_answer_is_spoken() {
        assert_eq!(
            spoken(&["How many moons does Mars have? 2. Both are small."]),
            vec!["How many moons does Mars have?", "2. Both are small."]
        );
    }

    #[test]
    fn a_sentence_before_a_pause_is_spoken_without_waiting_for_more_text() {
        let mut pipeline = SentencePipeline::new();
        assert_eq!(
            pipeline.feed("Let me **check** that."),
            Vec::<String>::new()
        );
        assert_eq!(pipeline.settle(), vec!["Let me check that."]);
        assert_eq!(pipeline.feed(" Found it."), Vec::<String>::new());
        assert_eq!(pipeline.flush(), vec!["Found it."]);
    }

    #[test]
    fn flush_returns_unterminated_tail() {
        let mut pipeline = SentencePipeline::new();
        assert_eq!(pipeline.feed("Still speaking"), Vec::<String>::new());
        assert_eq!(pipeline.flush(), vec!["Still speaking"]);
    }
}
