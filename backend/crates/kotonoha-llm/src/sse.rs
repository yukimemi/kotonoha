//! Tiny SSE helpers shared by the streaming HTTP backends.

/// Find the next SSE event boundary in `buf` and return `(position,
/// separator_length)`.  Accepts both `\n\n` (spec) and `\r\n\r\n`
/// (some intermediaries).
pub(crate) fn find_event_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some((p, 4));
    }
    buf.windows(2).position(|w| w == b"\n\n").map(|p| (p, 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_lf_lf_boundary() {
        assert_eq!(find_event_boundary(b"data: a\n\ndata: b"), Some((7, 2)));
    }

    #[test]
    fn finds_crlf_crlf_boundary() {
        assert_eq!(find_event_boundary(b"data: a\r\n\r\nrest"), Some((7, 4)));
    }

    #[test]
    fn none_when_incomplete() {
        assert_eq!(find_event_boundary(b"data: partial"), None);
    }
}
