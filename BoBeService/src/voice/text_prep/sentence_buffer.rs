const ABBREVIATIONS: &[&str] = &[
    "mr.", "mrs.", "ms.", "dr.", "st.", "jr.", "sr.", "inc.", "ltd.", "co.", "ph.d.", "m.d.",
    "u.s.", "u.k.", "u.s.a.", "etc.", "vs.", "i.e.", "e.g.", "a.m.", "p.m.", "fig.",
];
const LIST_MARKER_MAX_DIGITS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sentence {
    pub(crate) text: String,
    pub(crate) starts_line: bool,
}

pub(crate) struct SentenceBuffer {
    buffer: String,
    starts_line: bool,
}

impl SentenceBuffer {
    pub(crate) fn new() -> Self {
        Self {
            buffer: String::new(),
            starts_line: true,
        }
    }

    pub(crate) fn feed(&mut self, text: &str) -> Vec<Sentence> {
        self.buffer.push_str(text);
        self.release(false)
    }

    pub(crate) fn settle(&mut self) -> Vec<Sentence> {
        self.release(true)
    }

    pub(crate) fn flush(&mut self) -> Vec<Sentence> {
        let remaining = std::mem::take(&mut self.buffer);
        let starts_line = self.starts_line || begins_with_line_break(remaining.chars());
        let text = remaining.trim();
        if text.is_empty() {
            Vec::new()
        } else {
            vec![Sentence {
                text: text.to_owned(),
                starts_line,
            }]
        }
    }

    fn release(&mut self, paused: bool) -> Vec<Sentence> {
        let chars: Vec<char> = self.buffer.chars().collect();
        let mut sentences = Vec::new();
        let mut start = 0_usize;
        let mut i = 0_usize;

        while i < chars.len() {
            let c = chars[i];
            if !matches!(c, '.' | '!' | '?' | '\u{3002}' | '\u{FF01}' | '\u{FF1F}') {
                i += 1;
                continue;
            }
            let mut end = i;
            while chars.get(end + 1).is_some_and(|c| is_closing_mark(*c)) {
                end += 1;
            }
            let boundary = if c.is_ascii() {
                boundary_after(chars.get(end + 1..).unwrap_or_default())
            } else {
                Boundary::Here
            };
            match boundary {
                Boundary::Undecided if !paused => break,
                Boundary::None => {
                    i += 1;
                    continue;
                }
                Boundary::Undecided | Boundary::Here => {}
            }

            let segment: String = chars[start..=i].iter().collect();
            if ends_with_abbreviation(&segment.to_lowercase()) || is_list_marker(segment.trim()) {
                i += 1;
                continue;
            }

            let text: String = chars[start..=end].iter().collect();
            sentences.push(Sentence {
                text: text.trim().to_owned(),
                starts_line: self.starts_line
                    || begins_with_line_break(chars[start..=end].iter().copied()),
            });
            i = end + 1;
            self.starts_line = false;
            while i < chars.len() && chars[i].is_whitespace() {
                self.starts_line |= chars[i] == '\n';
                i += 1;
            }
            start = i;
        }

        self.buffer = chars[start..].iter().collect();
        sentences
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Boundary {
    Here,
    None,
    Undecided,
}

fn boundary_after(rest: &[char]) -> Boundary {
    let Some(first) = rest.first() else {
        return Boundary::Undecided;
    };
    if !first.is_whitespace() {
        return Boundary::None;
    }
    let mut index = 0;
    while let Some(c) = rest.get(index) {
        if *c == '\n' {
            return Boundary::Here;
        }
        if !c.is_whitespace() {
            break;
        }
        index += 1;
    }
    let Some(next) = rest.get(index) else {
        return Boundary::Undecided;
    };
    if next.is_uppercase() || matches!(next, '"' | '\'' | '\u{201C}' | '\u{2018}') {
        return Boundary::Here;
    }
    if !next.is_ascii_digit() {
        return Boundary::None;
    }
    let digits = rest[index..]
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .count();
    match (rest.get(index + digits), rest.get(index + digits + 1)) {
        (None, _) | (Some('.'), None) => Boundary::Undecided,
        (Some('.'), Some(after)) if after.is_whitespace() && digits <= LIST_MARKER_MAX_DIGITS => {
            Boundary::Here
        }
        _ => Boundary::None,
    }
}

fn begins_with_line_break(mut chars: impl Iterator<Item = char>) -> bool {
    chars
        .by_ref()
        .take_while(|c| c.is_whitespace())
        .any(|c| c == '\n')
}

fn ends_with_abbreviation(segment: &str) -> bool {
    ABBREVIATIONS.iter().any(|abbreviation| {
        segment.strip_suffix(abbreviation).is_some_and(|before| {
            before
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric())
        })
    })
}

fn is_closing_mark(c: char) -> bool {
    matches!(
        c,
        '*' | '_'
            | '`'
            | '"'
            | '\''
            | ')'
            | ']'
            | '\u{201D}'
            | '\u{2019}'
            | '\u{300D}'
            | '\u{300F}'
            | '\u{FF09}'
    )
}

fn is_list_marker(segment: &str) -> bool {
    segment.strip_suffix('.').is_some_and(|digits| {
        (1..=LIST_MARKER_MAX_DIGITS).contains(&digits.len())
            && digits.chars().all(|c| c.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sentences: Vec<Sentence>) -> Vec<String> {
        sentences
            .into_iter()
            .map(|sentence| sentence.text)
            .collect()
    }

    fn released(chunks: &[&str]) -> Vec<Sentence> {
        let mut buffer = SentenceBuffer::new();
        let mut sentences = Vec::new();
        for chunk in chunks {
            sentences.extend(buffer.feed(chunk));
        }
        sentences.extend(buffer.flush());
        sentences
    }

    fn spoken(chunks: &[&str]) -> Vec<String> {
        texts(released(chunks))
    }

    #[test]
    fn a_sentence_is_emitted_once_the_next_one_begins() {
        let mut b = SentenceBuffer::new();
        assert_eq!(b.feed("Hello world."), []);
        assert_eq!(texts(b.feed(" Next")), vec!["Hello world."]);
        assert_eq!(texts(b.flush()), vec!["Next"]);
    }

    #[test]
    fn a_finished_sentence_is_released_when_the_stream_pauses() {
        let mut b = SentenceBuffer::new();
        assert_eq!(b.feed("Let me check that."), []);
        assert_eq!(texts(b.settle()), vec!["Let me check that."]);
        assert_eq!(b.settle(), []);
        assert_eq!(b.feed(" Here it is."), []);
        assert_eq!(texts(b.flush()), vec!["Here it is."]);
    }

    #[test]
    fn a_pause_does_not_release_unfinished_text() {
        for unfinished in ["Still going", "Talk to Dr.", "1.", "Wait, e.g."] {
            let mut b = SentenceBuffer::new();
            assert_eq!(b.feed(unfinished), []);
            assert!(b.settle().is_empty(), "{unfinished}");
        }
    }

    #[test]
    fn buffers_text_without_terminator() {
        let mut b = SentenceBuffer::new();
        assert_eq!(b.feed("Hello world"), []);
        assert_eq!(b.feed(", and more."), []);
        assert_eq!(texts(b.flush()), vec!["Hello world, and more."]);
    }

    #[test]
    fn words_that_only_end_like_an_abbreviation_still_end_a_sentence() {
        assert_eq!(
            spoken(&["That is the best. Check those items. I said no. Then we left."]),
            vec![
                "That is the best.",
                "Check those items.",
                "I said no.",
                "Then we left."
            ]
        );
        assert_eq!(
            spoken(&["We met on Main St. Louis was there."]),
            vec!["We met on Main St. Louis was there."]
        );
    }

    #[test]
    fn a_bare_number_inside_a_paragraph_is_kept_with_its_sentence() {
        assert_eq!(
            spoken(&["How many moons? 2. Mars has two."]),
            vec!["How many moons?", "2. Mars has two."]
        );
        assert_eq!(
            spoken(&["What year? 1989. It was a big year."]),
            vec!["What year? 1989.", "It was a big year."]
        );
    }

    #[test]
    fn a_number_after_an_unlisted_abbreviation_does_not_split() {
        assert_eq!(
            spoken(&["See vol. 2 for details. It takes approx. 5 minutes."]),
            vec!["See vol. 2 for details.", "It takes approx. 5 minutes."]
        );
    }

    #[test]
    fn sentences_know_whether_they_start_a_line() {
        let sentences = released(&["1. First point. More on it.\n2. Second point."]);
        assert_eq!(
            sentences
                .iter()
                .map(|sentence| sentence.starts_line)
                .collect::<Vec<_>>(),
            vec![true, false, true]
        );
        let mut b = SentenceBuffer::new();
        assert_eq!(b.feed("One."), []);
        assert_eq!(b.settle().len(), 1);
        assert_eq!(b.feed("\n2. Two."), []);
        assert!(b.flush().iter().all(|sentence| sentence.starts_line));
    }

    #[test]
    fn splits_multiple_sentences_in_one_feed() {
        assert_eq!(
            spoken(&["How are you? I am fine! Goodbye."]),
            vec!["How are you?", "I am fine!", "Goodbye."]
        );
    }

    #[test]
    fn preserves_abbreviations_and_initialisms() {
        assert_eq!(
            spoken(&["Dr. Smith was here", " today. He left."]),
            vec!["Dr. Smith was here today.", "He left."]
        );
        assert_eq!(
            spoken(&["Travel to U.S. cities is fun. Try it."]),
            vec!["Travel to U.S. cities is fun.", "Try it."]
        );
    }

    #[test]
    fn a_decimal_split_across_deltas_stays_whole() {
        assert_eq!(
            spoken(&["Pi is 3", ".", "14 approximately. Cool."]),
            vec!["Pi is 3.14 approximately.", "Cool."]
        );
    }

    #[test]
    fn a_list_number_stays_with_its_sentence() {
        assert_eq!(
            spoken(&[
                "1",
                ".",
                " The Moon is close.",
                "\n2",
                ".",
                " It has no air."
            ]),
            vec!["1. The Moon is close.", "2. It has no air."]
        );
        assert_eq!(
            spoken(&["1. The Moon is close. 2. It has no air."]),
            vec!["1. The Moon is close.", "2. It has no air."]
        );
    }

    #[test]
    fn a_list_number_after_a_trailing_space_line_break_starts_the_next_sentence() {
        assert_eq!(
            spoken(&[
                "1. The Moon is close.  \n2",
                ".",
                " It has no air.  \n3. Done."
            ]),
            vec!["1. The Moon is close.", "2. It has no air.", "3. Done."]
        );
    }

    #[test]
    fn a_line_break_after_punctuation_ends_the_sentence() {
        assert_eq!(
            spoken(&["Here is the plan.\nfirst we wait."]),
            vec!["Here is the plan.", "first we wait."]
        );
    }

    #[test]
    fn unspaced_scripts_split_on_their_own_punctuation() {
        assert_eq!(
            spoken(&[
                "\u{4eca}\u{65e5}\u{306f}\u{6674}\u{308c}\u{3067}\u{3059}\u{3002}\u{6563}\u{6b69}\u{3057}\u{307e}\u{3059}\u{304b}\u{ff1f}"
            ]),
            vec![
                "\u{4eca}\u{65e5}\u{306f}\u{6674}\u{308c}\u{3067}\u{3059}\u{3002}",
                "\u{6563}\u{6b69}\u{3057}\u{307e}\u{3059}\u{304b}\u{ff1f}"
            ]
        );
    }

    #[test]
    fn closing_marks_stay_with_the_sentence_they_close() {
        assert_eq!(
            spoken(&["She said \"stop.\" Then she left."]),
            vec!["She said \"stop.\"", "Then she left."]
        );
        assert_eq!(
            spoken(&["**Hello.", "** Next one."]),
            vec!["**Hello.**", "Next one."]
        );
    }

    #[test]
    fn flush_drains_unterminated_remainder() {
        let mut b = SentenceBuffer::new();
        assert_eq!(b.feed("One sentence remaining no terminator"), []);
        assert_eq!(
            texts(b.flush()),
            vec!["One sentence remaining no terminator"]
        );
        assert_eq!(b.flush(), []);
    }

    #[test]
    fn handles_emdash_and_questions_together() {
        assert_eq!(
            spoken(&["Wait \u{2014} really? Yes!"]),
            vec!["Wait \u{2014} really?", "Yes!"]
        );
    }

    #[test]
    fn streamed_in_chunks_keeps_order_and_spacing() {
        assert_eq!(
            spoken(&["Hello", " there. ", "How are", " you? ", "All good!"]),
            vec!["Hello there.", "How are you?", "All good!"]
        );
    }
}
