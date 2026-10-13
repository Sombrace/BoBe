use std::sync::LazyLock;

use regex::Regex;

static CANCEL_PHRASE_RE: LazyLock<Regex> = LazyLock::new(|| {
    #[allow(clippy::expect_used)]
    Regex::new(
        r"(?ix)
        \b(
            stop                   |
            never\s*mind           |
            cancel(\s+that)?       |
            shut\s*up              |
            wait\s+wait            |
            pause\s+it             |
            hold\s+on\s+hold\s+on  |
            forget\s+it            |
            be\s+quiet
        )\b",
    )
    .expect("cancel-phrase regex must compile")
});

pub(crate) fn is_cancel_phrase(partial: &str) -> bool {
    CANCEL_PHRASE_RE.is_match(partial)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_canonical_cancels() {
        assert!(is_cancel_phrase("stop"));
        assert!(is_cancel_phrase("Stop please"));
        assert!(is_cancel_phrase("never mind"));
        assert!(is_cancel_phrase("nevermind"));
        assert!(is_cancel_phrase("cancel that"));
        assert!(is_cancel_phrase("just cancel"));
        assert!(is_cancel_phrase("shut up"));
        assert!(is_cancel_phrase("wait wait"));
        assert!(is_cancel_phrase("hold on hold on"));
        assert!(is_cancel_phrase("forget it"));
        assert!(is_cancel_phrase("be quiet please"));
    }

    #[test]
    fn does_not_match_non_cancels() {
        assert!(!is_cancel_phrase("hello world"));
        assert!(!is_cancel_phrase("tell me about apples"));
        assert!(!is_cancel_phrase("what was that"));
        assert!(!is_cancel_phrase("I would never"));
    }
}
