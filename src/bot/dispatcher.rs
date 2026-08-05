use matrix_sdk::ruma::UserId;

use crate::modules::CommandContext;

pub struct CommandParser;

impl CommandParser {
    pub fn parse_message(
        body: &str,
        formatted_body: Option<&str>,
        own_id: &UserId,
        room_id: &str,
        sender_mxid: &str,
        sender_localpart: &str,
    ) -> Option<CommandContext> {
        let body = body.trim();
        if body.is_empty() {
            return None;
        }

        let localpart = own_id.localpart();
        let full_mxid = own_id.to_string();
        let short_mention = format!("@{}", localpart);

        let body_lower = body.to_lowercase();
        let localpart_lower = localpart.to_lowercase();

        let is_html_mentioned = formatted_body
            .map(|f| f.contains(&full_mxid) || f.contains(&short_mention))
            .unwrap_or(false);

        let is_plain_mentioned = body.contains(&full_mxid)
            || body_lower.contains(&short_mention.to_lowercase())
            || body_lower.starts_with(&format!("{}:", localpart_lower))
            || body_lower.starts_with(&format!("{},", localpart_lower))
            || body_lower.starts_with(&format!("@{}", localpart_lower))
            || body_lower == localpart_lower;

        let is_mentioned = is_plain_mentioned || is_html_mentioned;
        let is_command_prefix = body.starts_with('!');

        if !is_command_prefix && !is_mentioned {
            return None;
        }

        let mut clean_body = body
            .replace(&full_mxid, "")
            .replace(&short_mention, "")
            .trim()
            .to_string();

        let clean_lower = clean_body.to_lowercase();
        if clean_lower.starts_with(&format!("{}:", localpart_lower))
            || clean_lower.starts_with(&format!("{},", localpart_lower))
        {
            clean_body = clean_body[localpart.len() + 1..].trim().to_string();
        } else if clean_lower.starts_with(&localpart_lower) {
            clean_body = clean_body[localpart.len()..].trim().to_string();
        }

        clean_body = clean_body
            .trim_start_matches(|c: char| c == ':' || c == ',' || c == '@' || c == ' ')
            .trim()
            .to_string();

        let mut parts = clean_body.splitn(2, ' ');
        let raw_command = parts.next().unwrap_or("").trim_start_matches('!').to_string();
        let raw_args = parts.next().unwrap_or("").trim().to_string();

        Some(CommandContext {
            room_id: room_id.to_string(),
            sender_mxid: sender_mxid.to_string(),
            sender_name: sender_localpart.to_string(),
            command: raw_command,
            raw_args,
            full_body: clean_body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message_exclamation_command() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let ctx = CommandParser::parse_message(
            "!todo add Buy milk",
            None,
            &bot_id,
            "!room:matrix.org",
            "@alice:matrix.org",
            "alice",
        )
        .unwrap();

        assert_eq!(ctx.command, "todo");
        assert_eq!(ctx.raw_args, "add Buy milk");
        assert_eq!(ctx.sender_name, "alice");
    }

    #[test]
    fn test_parse_message_mention_command() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let ctx = CommandParser::parse_message(
            "@hellobot remind 10s test",
            None,
            &bot_id,
            "!room:matrix.org",
            "@bob:matrix.org",
            "bob",
        )
        .unwrap();

        assert_eq!(ctx.command, "remind");
        assert_eq!(ctx.raw_args, "10s test");
        assert_eq!(ctx.sender_name, "bob");
    }

    #[test]
    fn test_parse_non_command_message() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let res = CommandParser::parse_message(
            "Just a random chatter message without bot mention",
            None,
            &bot_id,
            "!room:matrix.org",
            "@charlie:matrix.org",
            "charlie",
        );
        assert!(res.is_none());
    }
}
