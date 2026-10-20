pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        difference |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

pub(crate) fn hex_decode(input: &str) -> Option<Vec<u8>> {
    if !input.len().is_multiple_of(2) {
        return None;
    }
    input
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[high, low]| {
            let high = char::from(high).to_digit(16)?;
            let low = char::from(low).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

pub(crate) fn truncate_str(s: &str, max_bytes: usize) -> &str {
    &s[..s.floor_char_boundary(max_bytes)]
}

pub(crate) fn tail_str(s: &str, max_bytes: usize) -> &str {
    &s[s.ceil_char_boundary(s.len().saturating_sub(max_bytes))..]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{constant_time_eq, hex_decode, hex_encode, tail_str, truncate_str};

    #[test]
    fn tail_keeps_the_end_on_a_character_boundary() {
        assert_eq!(tail_str("hello", 10), "hello");
        assert_eq!(tail_str("hello world", 5), "world");
        assert_eq!(tail_str("caf\u{e9}s", 2), "s");
        assert_eq!(tail_str("caf\u{e9}s", 3), "\u{e9}s");
    }

    #[test]
    fn constant_time_comparison_handles_equal_and_different_lengths() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"same", b"same-longer"));
    }

    #[test]
    fn hexadecimal_encoding_is_lowercase_and_padded() {
        let bytes = [0, 1, 15, 16, 255];
        assert_eq!(hex_encode(&bytes), "00010f10ff");
        assert_eq!(hex_decode("00010F10ff"), Some(bytes.to_vec()));
        assert_eq!(hex_decode("0"), None);
        assert_eq!(hex_decode("gg"), None);
    }

    #[test]
    fn truncation_preserves_utf8_boundaries() {
        for (input, limit, expected) in [
            ("hello", 10, "hello"),
            ("hello world", 5, "hello"),
            ("", 100, ""),
            ("a€b", 2, "a"),
            ("a€b", 4, "a€"),
            ("hi👋ok", 3, "hi"),
            ("hi👋ok", 6, "hi👋"),
            ("hello", 0, ""),
        ] {
            assert_eq!(truncate_str(input, limit), expected);
        }
    }
}
