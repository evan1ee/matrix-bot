//! Rooms: the Matrix rooms an account can be reached in, and the rules that
//! pick one when an action names no room or names it loosely.
//!
//! Resolution is decided here, over data the application service gathers, so
//! the same rules answer for the web UI, the Matrix bot, LLM tools and
//! background schedulers.

use std::cmp::Ordering;

/// The room id an action falls back to when no Matrix room can be resolved:
/// the item is kept for the web UI and delivered nowhere in Matrix.
pub const WEB_ROOM: &str = "web";

/// Placeholders a client sends to mean "no particular room" rather than a
/// room called "web".
const NO_ROOM_PLACEHOLDERS: [&str; 2] = [WEB_ROOM, "web_chat"];

/// A Matrix room that both the bot and one of the account's linked Matrix
/// identities have joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedRoom {
    room_id: String,
    name: String,
    is_direct: bool,
}

impl JoinedRoom {
    /// Builds a room from whatever name the homeserver offered.
    ///
    /// The name is shown in the room picker and matched by name lookups, so it
    /// is never blank: a missing or whitespace-only name falls back to the
    /// room id.
    pub fn new(room_id: impl Into<String>, name: Option<&str>, is_direct: bool) -> Self {
        let room_id = room_id.into();
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map_or_else(|| room_id.clone(), str::to_owned);
        Self {
            room_id,
            name,
            is_direct,
        }
    }

    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_direct(&self) -> bool {
        self.is_direct
    }

    /// The order the room picker shows rooms in: direct chats first, then by
    /// name, ignoring case. The repository's `ORDER BY` mirrors it, so live
    /// discovery and the cache list rooms identically.
    pub fn picker_order(a: &Self, b: &Self) -> Ordering {
        b.is_direct
            .cmp(&a.is_direct)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    }
}

/// A joined room as remembered in the account's room cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedRoom {
    pub room: JoinedRoom,
    /// When the room was cached, as PostgreSQL's `NOW()::text`. Compared as
    /// text, which orders correctly because every value shares that format.
    pub updated_at: String,
}

/// What a caller asked for when it named a target room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomTarget<'a> {
    /// A Matrix room id (`!…`), used as given without consulting the cache.
    RoomId(&'a str),
    /// A room name or id to look up in the account's cached rooms.
    Named(&'a str),
    /// Nothing specific: deliver to the account's default room.
    UserDefault,
}

impl<'a> RoomTarget<'a> {
    pub fn parse(query: Option<&'a str>) -> Self {
        let trimmed = query.map(str::trim).unwrap_or("");
        if trimmed.is_empty() || NO_ROOM_PLACEHOLDERS.contains(&trimmed) {
            RoomTarget::UserDefault
        } else if trimmed.starts_with('!') {
            RoomTarget::RoomId(trimmed)
        } else {
            RoomTarget::Named(trimmed)
        }
    }
}

/// Resolves a loosely named room against the cache: an exact name (ignoring
/// case) or room id first, then a name containing `query`.
///
/// Falls back to `query` itself, unchanged, when nothing matches — the caller
/// stores what the user typed rather than silently redirecting the action to
/// another room.
pub fn match_named<'a>(query: &'a str, rooms: &'a [CachedRoom]) -> &'a str {
    let wanted = query.to_lowercase();
    let exact = rooms
        .iter()
        .find(|r| r.room.name.to_lowercase() == wanted || r.room.room_id == query);
    let partial = || {
        rooms
            .iter()
            .find(|r| r.room.name.to_lowercase().contains(&wanted))
    };
    exact
        .or_else(partial)
        .map_or(query, |r| r.room.room_id.as_str())
}

/// Picks the room an account is reached in when an action names none.
///
/// Priority, highest first:
/// 1. the most recently cached direct chat with the user;
/// 2. a cached room named after the localpart of one of the account's linked
///    Matrix ids (in link order) — a DM whose direct flag the client never
///    set is typically named after the other member;
/// 3. the dev room from the user's settings, if not blank;
/// 4. the most recently cached room of any kind.
pub fn default_room<'a>(
    rooms: &'a [CachedRoom],
    linked_mxids: &[String],
    dev_room_id: Option<&'a str>,
) -> Option<&'a str> {
    let most_recent = |direct_only: bool| {
        rooms
            .iter()
            .filter(|r| !direct_only || r.room.is_direct)
            .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
            .map(|r| r.room.room_id.as_str())
    };
    let named_after_user = || {
        linked_mxids.iter().find_map(|mxid| {
            let localpart = mxid_localpart(mxid).to_lowercase();
            rooms
                .iter()
                .find(|r| r.room.name.to_lowercase() == localpart)
                .map(|r| r.room.room_id.as_str())
        })
    };
    let dev_room = || dev_room_id.filter(|id| !id.trim().is_empty());

    most_recent(true)
        .or_else(named_after_user)
        .or_else(dev_room)
        .or_else(|| most_recent(false))
}

/// `@evan:mbm.one` → `evan`. Tolerates a missing sigil or server name, since
/// the value comes from storage rather than a freshly parsed Matrix id.
fn mxid_localpart(mxid: &str) -> &str {
    let without_sigil = mxid.strip_prefix('@').unwrap_or(mxid);
    without_sigil
        .split_once(':')
        .map_or(without_sigil, |(localpart, _)| localpart)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached(room_id: &str, name: &str, is_direct: bool, updated_at: &str) -> CachedRoom {
        CachedRoom {
            room: JoinedRoom::new(room_id, Some(name), is_direct),
            updated_at: updated_at.to_string(),
        }
    }

    fn mxids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_room_name_is_trimmed() {
        let room = JoinedRoom::new("!a:x", Some("  Dev Channel "), false);
        assert_eq!(room.name(), "Dev Channel");
    }

    #[test]
    fn a_missing_or_blank_name_falls_back_to_the_room_id() {
        assert_eq!(JoinedRoom::new("!a:x", None, false).name(), "!a:x");
        assert_eq!(JoinedRoom::new("!a:x", Some("   "), true).name(), "!a:x");
    }

    #[test]
    fn the_picker_lists_direct_rooms_first_then_by_name_ignoring_case() {
        let mut rooms = vec![
            JoinedRoom::new("!b:x", Some("general"), false),
            JoinedRoom::new("!c:x", Some("Zed"), true),
            JoinedRoom::new("!a:x", Some("DEV"), false),
        ];
        rooms.sort_by(JoinedRoom::picker_order);
        let names: Vec<&str> = rooms.iter().map(JoinedRoom::name).collect();
        assert_eq!(names, ["Zed", "DEV", "general"]);
    }

    #[test]
    fn a_blank_or_placeholder_target_means_the_user_default() {
        for query in [None, Some(""), Some("  "), Some("web"), Some(" web_chat ")] {
            assert_eq!(
                RoomTarget::parse(query),
                RoomTarget::UserDefault,
                "{query:?}"
            );
        }
    }

    #[test]
    fn a_target_starting_with_a_bang_is_a_room_id() {
        assert_eq!(
            RoomTarget::parse(Some(" !abc:x ")),
            RoomTarget::RoomId("!abc:x")
        );
    }

    #[test]
    fn any_other_target_is_a_name_to_look_up() {
        assert_eq!(RoomTarget::parse(Some(" Dev ")), RoomTarget::Named("Dev"));
    }

    #[test]
    fn a_named_target_prefers_an_exact_name_over_a_partial_one() {
        let rooms = [
            cached("!long:x", "Dev Channel", false, "1"),
            cached("!exact:x", "dev", false, "1"),
        ];
        assert_eq!(match_named("DEV", &rooms), "!exact:x");
    }

    #[test]
    fn a_named_target_matches_a_room_id_exactly() {
        let rooms = [cached("#alias:x", "Ops", false, "1")];
        assert_eq!(match_named("#alias:x", &rooms), "#alias:x");
    }

    #[test]
    fn a_named_target_falls_back_to_a_partial_name_match() {
        let rooms = [cached("!dev:x", "Dev Channel", false, "1")];
        assert_eq!(match_named("chan", &rooms), "!dev:x");
    }

    #[test]
    fn an_unmatched_name_is_kept_as_typed() {
        let rooms = [cached("!dev:x", "Dev Channel", false, "1")];
        assert_eq!(match_named("ops", &rooms), "ops");
    }

    #[test]
    fn the_default_is_the_most_recent_direct_room() {
        let rooms = [
            cached("!group:x", "evan", false, "2026-09-03"),
            cached("!old_dm:x", "a", true, "2026-09-01"),
            cached("!new_dm:x", "b", true, "2026-09-02"),
        ];
        assert_eq!(
            default_room(&rooms, &mxids(&["@evan:x"]), Some("!dev:x")),
            Some("!new_dm:x")
        );
    }

    #[test]
    fn without_a_direct_room_the_default_is_the_room_named_after_the_user() {
        let rooms = [
            cached("!newer:x", "general", false, "2026-09-02"),
            cached("!named:x", "Evan", false, "2026-09-01"),
        ];
        assert_eq!(
            default_room(
                &rooms,
                &mxids(&["@nobody:x", "@evan:mbm.one"]),
                Some("!dev:x")
            ),
            Some("!named:x")
        );
    }

    #[test]
    fn then_the_dev_room_from_settings() {
        let rooms = [cached("!general:x", "general", false, "1")];
        assert_eq!(
            default_room(&rooms, &mxids(&["@evan:x"]), Some("!dev:x")),
            Some("!dev:x")
        );
    }

    #[test]
    fn a_blank_dev_room_is_skipped_for_the_most_recent_room() {
        let rooms = [
            cached("!older:x", "a", false, "2026-09-01"),
            cached("!newer:x", "b", false, "2026-09-02"),
        ];
        assert_eq!(default_room(&rooms, &[], Some("  ")), Some("!newer:x"));
    }

    #[test]
    fn with_nothing_cached_or_configured_there_is_no_default() {
        assert_eq!(default_room(&[], &mxids(&["@evan:x"]), None), None);
    }

    #[test]
    fn a_localpart_tolerates_a_missing_sigil_or_server() {
        assert_eq!(mxid_localpart("@evan:mbm.one"), "evan");
        assert_eq!(mxid_localpart("evan:mbm.one"), "evan");
        assert_eq!(mxid_localpart("@evan"), "evan");
    }
}
