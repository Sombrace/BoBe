use std::sync::LazyLock;

use regex::Regex;

#[allow(
    clippy::expect_used,
    reason = "static compile-time regexes; failure is a developer error caught in tests"
)]
static MD_RE: LazyLock<MarkdownRegexes> = LazyLock::new(|| MarkdownRegexes {
    italic_star: Regex::new(r"\*([^*\n]+)\*").expect("static italic-star regex"),
    italic_underscore: Regex::new(r"_([^_\n]+)_").expect("static italic-underscore regex"),
    header: Regex::new(r"(?m)^#{1,6}\s+").expect("static header regex"),
    bullet: Regex::new(r"(?m)^\s*[-*•]\s+").expect("static bullet regex"),
    numbered: Regex::new(r"(?m)^\s*\d{1,3}\.\s+").expect("static numbered-list regex"),
    link: Regex::new(r"\[([^\]]+)\]\([^)]+\)").expect("static markdown-link regex"),
});

struct MarkdownRegexes {
    italic_star: Regex,
    italic_underscore: Regex,
    header: Regex,
    bullet: Regex,
    numbered: Regex,
    link: Regex,
}

#[derive(Default)]
pub(crate) struct FenceFilter {
    in_fence: bool,
    backticks: usize,
}

impl FenceFilter {
    pub(crate) fn feed(&mut self, delta: &str) -> String {
        let mut visible = String::with_capacity(delta.len());
        for c in delta.chars() {
            if c == '`' {
                self.backticks += 1;
                if self.backticks == 3 {
                    self.in_fence = !self.in_fence;
                    self.backticks = 0;
                }
                continue;
            }
            self.release_backticks(&mut visible);
            if !self.in_fence {
                visible.push(c);
            }
        }
        visible
    }

    pub(crate) fn flush(&mut self) -> String {
        let mut visible = String::new();
        self.release_backticks(&mut visible);
        visible
    }

    fn release_backticks(&mut self, visible: &mut String) {
        if !self.in_fence {
            visible.extend(std::iter::repeat_n('`', self.backticks));
        }
        self.backticks = 0;
    }
}

pub(crate) fn strip_markdown(text: &str, starts_line: bool) -> String {
    let re = &*MD_RE;
    let mut out = text.replace("**", "").replace("__", "").replace('`', "");
    out = re.italic_star.replace_all(&out, "$1").into_owned();
    out = re.italic_underscore.replace_all(&out, "$1").into_owned();
    out = re.link.replace_all(&out, "$1").into_owned();
    if starts_line {
        return strip_line_markers(&out);
    }
    match out.split_once('\n') {
        Some((first_line, rest)) => format!("{first_line}\n{}", strip_line_markers(rest)),
        None => out,
    }
}

fn strip_line_markers(text: &str) -> String {
    let re = &*MD_RE;
    let out = re.header.replace_all(text, "");
    let out = re.bullet.replace_all(&out, "");
    re.numbered.replace_all(&out, "").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_bold_and_italic() {
        assert_eq!(strip_markdown("**Hello** _world_", true), "Hello world");
        assert_eq!(
            strip_markdown("a *very* **big** deal", false),
            "a very big deal"
        );
    }

    #[test]
    fn strips_headers_bullets_links() {
        let md = "# Title\n- one\n- two\n[click](https://x)";
        assert_eq!(strip_markdown(md, true), "Title\none\ntwo\nclick");
    }

    #[test]
    fn line_markers_are_only_removed_where_a_line_starts() {
        assert_eq!(strip_markdown("2. Mars has two.", true), "Mars has two.");
        assert_eq!(
            strip_markdown("2. Mars has two.", false),
            "2. Mars has two."
        );
        assert_eq!(
            strip_markdown("- 4 is negative.", false),
            "- 4 is negative."
        );
        assert_eq!(
            strip_markdown("Options:\n- one\n2. two", false),
            "Options:\none\ntwo"
        );
        assert_eq!(
            strip_markdown("1989. It was big.", true),
            "1989. It was big."
        );
    }

    #[test]
    fn fenced_code_is_removed() {
        let mut filter = FenceFilter::default();
        assert_eq!(
            filter.feed("before\n```rust\nlet x = 1;\n```\nafter"),
            "before\n\nafter"
        );
    }

    #[test]
    fn streaming_suppresses_open_fence() {
        let mut filter = FenceFilter::default();
        assert_eq!(
            filter.feed("Let me show you:\n```rust\nlet"),
            "Let me show you:\n"
        );
        assert_eq!(filter.feed("x = 1;\n"), "");
        assert_eq!(filter.feed("```\nThat's it."), "\nThat's it.");
    }

    #[test]
    fn a_fence_marker_split_across_deltas_never_shifts_later_text() {
        let mut filter = FenceFilter::default();
        let mut visible = filter.feed("A ``");
        visible.push_str(&filter.feed("`code``` B c"));
        visible.push_str(&filter.flush());
        assert_eq!(visible, "A  B c");
    }

    #[test]
    fn inline_code_marks_pass_through_until_the_sentence_is_cleaned() {
        let mut filter = FenceFilter::default();
        let mut visible = filter.feed("Run `ls");
        visible.push_str(&filter.feed("` now"));
        visible.push_str(&filter.flush());
        assert_eq!(visible, "Run `ls` now");
        assert_eq!(strip_markdown(&visible, false), "Run ls now");
    }

    #[test]
    fn emphasis_left_open_by_a_sentence_break_is_not_spoken() {
        assert_eq!(strip_markdown("**Note this.", true), "Note this.");
        assert_eq!(strip_markdown("It matters.**", false), "It matters.");
    }

    #[test]
    fn handles_consecutive_fences() {
        let mut filter = FenceFilter::default();
        let out =
            filter.feed("intro\n```rust\nlet x = 1;\n```\nmiddle\n```python\nprint(1)\n```\nend");
        assert_eq!(out, "intro\n\nmiddle\n\nend");
    }

    #[test]
    fn streaming_suppresses_until_second_fence_closes() {
        let mut filter = FenceFilter::default();
        assert_eq!(filter.feed("Run ```sh\necho hi\n``` then "), "Run  then ");
        assert_eq!(filter.feed("```sh\necho bye\n"), "");
        assert_eq!(filter.feed("``` and exit"), " and exit");
    }

    #[test]
    fn flat_asterisk_runs_unchanged() {
        let out = strip_markdown("5 * 4 = 20", false);
        assert!(out.contains('5'), "{out}");
        assert!(out.contains("20"), "{out}");
    }

    #[test]
    fn link_preserves_link_text_and_drops_url() {
        let out = strip_markdown("Visit [our docs](https://example.com/docs) please", false);
        assert_eq!(out, "Visit our docs please");
        assert!(!out.contains("https://"), "{out}");
    }
}
