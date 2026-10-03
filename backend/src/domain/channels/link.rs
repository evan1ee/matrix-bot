//! Linking a platform identity to an account: the pure decision.
//!
//! The proof of identity ownership is the direct message that minted the link
//! token; the proof of account ownership is the passkey that claims it. What
//! is left to decide, given who the identity maps to now, is only this.

use chrono::Duration;

use crate::domain::accounts::AccountId;

/// How long a link token stays claimable: long enough to switch to a browser
/// and sign in, short enough that a token left in a chat is not a standing
/// invitation.
pub const LINK_TOKEN_TTL: Duration = Duration::minutes(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkDecision {
    /// Nobody holds the identity: bind it.
    Bind,
    /// It is already on the claiming account.
    AlreadyYours,
    /// An auto-provisioned (unclaimed) account holds it: absorb that account.
    Merge { from: AccountId },
    /// A claimed account holds it; it belongs to someone and is never taken.
    OwnedElsewhere,
}

/// `current` is the account the identity maps to now, if any, and whether that
/// account is unclaimed (auto-provisioned, no passkey ever registered).
pub fn decide_link(viewer: &AccountId, current: Option<(&AccountId, bool)>) -> LinkDecision {
    match current {
        None => LinkDecision::Bind,
        Some((holder, _)) if holder == viewer => LinkDecision::AlreadyYours,
        Some((holder, true)) => LinkDecision::Merge { from: *holder },
        Some((_, false)) => LinkDecision::OwnedElsewhere,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;

    #[test]
    fn link_decisions() {
        let me = AccountId::from_seed("me");
        let other = AccountId::from_seed("other");
        assert_eq!(decide_link(&me, None), LinkDecision::Bind);
        assert_eq!(
            decide_link(&me, Some((&me, false))),
            LinkDecision::AlreadyYours
        );
        assert_eq!(
            decide_link(&me, Some((&me, true))),
            LinkDecision::AlreadyYours,
            "an identity already on my account is mine, whatever its status"
        );
        assert_eq!(
            decide_link(&me, Some((&other, true))),
            LinkDecision::Merge { from: other }
        );
        assert_eq!(
            decide_link(&me, Some((&other, false))),
            LinkDecision::OwnedElsewhere
        );
    }

    #[test]
    fn a_link_token_lives_ten_minutes() {
        assert_eq!(LINK_TOKEN_TTL, chrono::Duration::minutes(10));
    }
}
