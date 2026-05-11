//! Case-insensitive subsequence matching used by both the `/` filter and the
//! per-character highlight in the task row renderer.

/// Returns byte offsets in `haystack` where each char of `needle` is matched
/// in order, case-insensitively, with arbitrary gaps allowed. Returns `None`
/// when not every needle char can be matched, or when `needle` is empty.
///
/// Offsets are into the original `haystack` (not a lowercased copy), so they
/// land on `char_indices` boundaries and are safe to slice.
pub fn subseq_match_ci(haystack: &str, needle: &str) -> Option<Vec<usize>> {
    if needle.is_empty() {
        return None;
    }
    let needle_lower: Vec<String> = needle
        .chars()
        .map(|c| c.to_lowercase().collect::<String>())
        .collect();
    let mut positions = Vec::with_capacity(needle_lower.len());
    let mut idx = 0;
    for (byte, ch) in haystack.char_indices() {
        if idx == needle_lower.len() {
            break;
        }
        let ch_lower: String = ch.to_lowercase().collect();
        if ch_lower == needle_lower[idx] {
            positions.push(byte);
            idx += 1;
        }
    }
    (idx == needle_lower.len()).then_some(positions)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn matches_contiguous_substring() {
        assert_eq!(subseq_match_ci("Hello", "ell"), Some(vec![1, 2, 3]));
    }

    #[test]
    fn matches_subsequence_with_gaps() {
        // The motivating bug: "cade" finds C, a, d, e in "Call dentist".
        let positions = subseq_match_ci("Call dentist", "cade").unwrap();
        assert_eq!(positions, vec![0, 1, 5, 6]);
    }

    #[test]
    fn case_insensitive_both_directions() {
        assert_eq!(subseq_match_ci("HELLO", "ell"), Some(vec![1, 2, 3]));
        assert_eq!(subseq_match_ci("hello", "ELL"), Some(vec![1, 2, 3]));
    }

    #[test]
    fn empty_needle_is_none() {
        assert_eq!(subseq_match_ci("anything", ""), None);
    }

    #[test]
    fn missing_chars_return_none() {
        assert_eq!(subseq_match_ci("hello", "xyz"), None);
        // "cae" is a subsequence of "Call dentist" but "caz" is not.
        assert_eq!(subseq_match_ci("Call dentist", "caz"), None);
    }

    #[test]
    fn order_matters() {
        // Subsequence is in-order: "dc" can't match "Call dentist" because 'd'
        // appears after 'c'.
        assert_eq!(subseq_match_ci("Call dentist", "dc"), None);
    }

    #[test]
    fn offsets_land_on_char_boundaries_for_unicode() {
        // "Café" byte layout: C(0) a(1) f(2) é(3..5). Matching "cé" should
        // return byte offsets that the caller can slice without panicking.
        let positions = subseq_match_ci("Café", "cé").unwrap();
        assert_eq!(positions, vec![0, 3]);
        let haystack = "Café";
        for p in positions {
            // Will panic on a non-boundary slice; assert we're safe.
            let _ = &haystack[p..];
        }
    }
}
