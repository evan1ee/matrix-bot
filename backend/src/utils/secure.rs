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

/// Lowercase hex SHA-256 of `bytes`. The crate spells this out once, so
/// `domain/` can hash a token or an audit record without depending on a
/// crypto crate itself.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_the_fips_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

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
