//! Security helpers shared by more than one layer.

/// Compares two secrets without leaking, through timing, how many leading bytes
/// matched. `==` on slices returns at the first difference, so an attacker who can
/// time the webhook check could recover a secret one byte at a time. Only the
/// length is allowed to short-circuit: it is not secret, and the fold below needs
/// equal lengths. `#[inline(never)]` keeps the optimizer from fusing the fold with
/// a caller's branch and reintroducing an early exit.
#[inline(never)]
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_slices_match() {
        assert!(constant_time_eq(b"s3cret", b"s3cret"));
    }

    #[test]
    fn unequal_slices_of_the_same_length_differ() {
        assert!(!constant_time_eq(b"s3cret", b"s3creT"));
        assert!(!constant_time_eq(b"a3cret", b"s3cret"));
    }

    #[test]
    fn slices_of_different_lengths_differ() {
        assert!(!constant_time_eq(b"s3cret", b"s3cre"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn empty_slices_match() {
        assert!(constant_time_eq(b"", b""));
    }
}
