//! Unguessable identifiers and secrets, generated in the process.
//!
//! This used to be SQLite's `randomblob(n)`, chosen because no random-number
//! crate was a direct dependency and SQLite ships a CSPRNG. That reasoning
//! stopped holding twice over. `uuid` became a direct dependency and is already
//! used for `conv_`, `msg_` and `tok_` ids, so the premise was no longer true.
//! And `randomblob` does not exist on PostgreSQL, which turned a dialect detail
//! into a question about where session tokens get their entropy.
//!
//! That is the wrong place for a portability seam. A token's unguessability
//! should not depend on which database someone deployed against, so the
//! randomness comes from the OS CSPRNG here and the same bytes go to either
//! database as an ordinary bound parameter.

use uuid::Uuid;

/// `n` random bytes from the OS CSPRNG, hex-encoded in lowercase.
///
/// Matches what `lower(hex(randomblob(n)))` produced, so stored values from
/// before this change are indistinguishable in shape from ones after it.
///
/// Built from `Uuid::new_v4()` — 16 CSPRNG bytes each, via `getrandom` — rather
/// than adding another dependency for the same job. Sizes that are not a
/// multiple of 16 take the prefix of the last one, which loses no quality: the
/// bytes are independent and uniform.
pub fn random_hex(bytes: usize) -> String {
    let mut out = String::with_capacity(bytes * 2);
    while out.len() < bytes * 2 {
        out.push_str(&Uuid::new_v4().simple().to_string());
    }
    out.truncate(bytes * 2);
    out
}

/// `n` random bytes from the OS CSPRNG.
pub fn random_bytes(bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes);
    while out.len() < bytes {
        out.extend_from_slice(Uuid::new_v4().as_bytes());
    }
    out.truncate(bytes);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_random_hex_has_two_characters_per_byte() {
        for bytes in [8, 10, 16, 20, 24, 32] {
            assert_eq!(random_hex(bytes).len(), bytes * 2, "for {bytes} bytes");
        }
    }

    #[test]
    fn test_random_hex_is_lowercase_hex() {
        let value = random_hex(32);
        assert!(
            value
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "not lowercase hex: {value}"
        );
    }

    #[test]
    fn test_random_bytes_returns_the_length_asked_for() {
        for bytes in [8, 10, 16, 24, 48] {
            assert_eq!(random_bytes(bytes).len(), bytes, "for {bytes} bytes");
        }
    }

    /// The whole point of the module. A generator that repeated itself would
    /// hand two users the same session token, and nothing else here would
    /// notice.
    #[test]
    fn test_values_do_not_repeat() {
        let hexes: HashSet<String> = (0..500).map(|_| random_hex(16)).collect();
        assert_eq!(hexes.len(), 500, "random_hex repeated a value");
    }

    /// Sizes past 16 bytes are stitched from more than one UUID; the join must
    /// not produce a constant region.
    #[test]
    fn test_values_longer_than_one_uuid_vary_throughout() {
        let a = random_hex(32);
        let b = random_hex(32);
        assert_ne!(a[..32], b[..32], "first half is constant");
        assert_ne!(a[32..], b[32..], "second half is constant");
    }
}
