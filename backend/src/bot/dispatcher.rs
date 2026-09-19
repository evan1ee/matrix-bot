use matrix_sdk::ruma::UserId;

use crate::modules::ParsedCommand;

pub struct CommandParser;

impl CommandParser {
    /// Recognises a command without touching the database.
    ///
    /// Deliberately knows nothing about accounts: resolving the sender costs a
    /// query, and the overwhelming majority of messages the bot sees are
    /// ordinary room chatter that this function rejects. The caller resolves an
    /// owner only once something comes back `Some`.
    pub fn parse_message(
        body: &str,
        formatted_body: Option<&str>,
        own_id: &UserId,
        room_id: &str,
        sender_localpart: &str,
    ) -> Option<ParsedCommand> {
        let (quoted, prompt) = crate::bot::quote::extract_quote_and_prompt(body, formatted_body);
        let prompt_trimmed = prompt.trim();
        if prompt_trimmed.is_empty() && quoted.is_none() {
            return None;
        }

        let localpart = own_id.localpart();
        let full_mxid = own_id.to_string();
        let short_mention = format!("@{}", localpart);

        let body_lower = prompt_trimmed.to_lowercase();
        let localpart_lower = localpart.to_lowercase();

        let is_html_mentioned = formatted_body
            .map(|f| f.contains(&full_mxid) || f.contains(&short_mention))
            .unwrap_or(false);

        let is_plain_mentioned = prompt_trimmed.contains(&full_mxid)
            || body_lower.contains(&short_mention.to_lowercase())
            || body_lower.starts_with(&format!("{}:", localpart_lower))
            || body_lower.starts_with(&format!("{},", localpart_lower))
            || body_lower.starts_with(&format!("@{}", localpart_lower))
            || body_lower == localpart_lower;

        let is_mentioned = is_plain_mentioned || is_html_mentioned;
        let is_command_prefix = prompt_trimmed.starts_with('!');

        if !is_command_prefix && !is_mentioned {
            return None;
        }

        let mut clean_body = prompt_trimmed
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
            .trim_start_matches([':', ',', '@', ' '])
            .trim()
            .to_string();

        let (raw_command, raw_args) = if clean_body.is_empty() {
            if quoted.is_some() {
                ("ai".to_string(), String::new())
            } else {
                return None;
            }
        } else {
            let mut parts = clean_body.splitn(2, ' ');
            let cmd = parts
                .next()
                .unwrap_or("")
                .trim_start_matches('!')
                .to_string();
            let args = parts.next().unwrap_or("").trim().to_string();
            (cmd, args)
        };

        Some(ParsedCommand {
            room_id: room_id.to_string(),
            sender_name: sender_localpart.to_string(),
            command: raw_command,
            raw_args,
            full_body: clean_body,
            is_mentioned,
            quoted,
        })
    }

    /// Parses a message, falling back to treating a direct message as an AI
    /// prompt.
    ///
    /// Every other rule about what counts as a command lives here; this one was
    /// written inline at the call site, so the parser could not be asked "is
    /// this a command?" and give the right answer.
    pub fn parse_or_dm_prompt(
        body: &str,
        formatted_body: Option<&str>,
        own_id: &UserId,
        room_id: &str,
        sender_localpart: &str,
        is_direct: bool,
    ) -> Option<ParsedCommand> {
        if let Some(parsed) =
            Self::parse_message(body, formatted_body, own_id, room_id, sender_localpart)
        {
            return Some(parsed);
        }

        let trimmed = body.trim();
        if is_direct && !trimmed.is_empty() {
            let (quoted, prompt) =
                crate::bot::quote::extract_quote_and_prompt(trimmed, formatted_body);
            Some(ParsedCommand {
                room_id: room_id.to_string(),
                sender_name: sender_localpart.to_string(),
                command: "ai".to_string(),
                raw_args: prompt.clone(),
                full_body: prompt,
                is_mentioned: false,
                quoted,
            })
        } else {
            None
        }
    }

    pub fn is_ai_prompt_command(parsed: &ParsedCommand) -> bool {
        let cmd = parsed.command.to_ascii_lowercase();
        if cmd == "ai" || cmd == "ask" {
            let trimmed = parsed.raw_args.trim();
            let first_word = trimmed.split_whitespace().next().unwrap_or("");
            !matches!(
                first_word.to_ascii_lowercase().as_str(),
                "reset" | "clear" | "config" | "retro" | "retrospective" | "weekly"
            )
        } else if parsed.is_mentioned {
            const BUILTIN_COMMANDS: &[&str] = &[
                "ping",
                "help",
                "stat",
                "stats",
                "version",
                "uptime",
                "todo",
                "todos",
                "remind",
                "reminder",
                "reminders",
                "finance",
                "expense",
                "income",
                "budget",
                "portfolio",
                "invest",
                "stock",
                "crypto",
                "asset",
                "footprint",
                "footprints",
                "trip",
                "trips",
                "habit",
                "habits",
                "life",
                "lifegrid",
                "bill",
                "bills",
                "settings",
                "setting",
                "webhook",
                "webhooks",
                "link",
            ];
            !BUILTIN_COMMANDS.contains(&cmd.as_str())
        } else {
            false
        }
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
            "charlie",
        );
        assert!(res.is_none());
    }

    #[test]
    fn test_parse_quoted_reply_fact_check() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let msg =
            "> <@bob:matrix.org> The moon is made of green cheese.\n\n@hellobot fact check this";
        let ctx =
            CommandParser::parse_message(msg, None, &bot_id, "!room:matrix.org", "alice").unwrap();

        assert_eq!(ctx.command, "fact");
        assert_eq!(ctx.raw_args, "check this");
        assert_eq!(ctx.full_body, "fact check this");
        assert!(ctx.is_mentioned);
        assert!(ctx.quoted.is_some());
        let q = ctx.quoted.unwrap();
        assert_eq!(q.author.as_deref(), Some("@bob:matrix.org"));
        assert_eq!(q.body, "The moon is made of green cheese.");
    }

    #[test]
    fn test_parse_quoted_reply_add_todo() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let msg = "> Review security audit before Monday\n\n@hellobot add todo";
        let ctx =
            CommandParser::parse_message(msg, None, &bot_id, "!room:matrix.org", "alice").unwrap();

        assert_eq!(ctx.command, "add");
        assert_eq!(ctx.raw_args, "todo");
        assert!(ctx.quoted.is_some());
        let q = ctx.quoted.unwrap();
        assert_eq!(q.body, "Review security audit before Monday");
    }

    #[test]
    fn test_parse_quoted_reply_bare_mention() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let msg = "> Important news snippet\n\n@hellobot";
        let ctx =
            CommandParser::parse_message(msg, None, &bot_id, "!room:matrix.org", "alice").unwrap();

        assert_eq!(ctx.command, "ai");
        assert!(ctx.quoted.is_some());
        assert_eq!(ctx.quoted.unwrap().body, "Important news snippet");
    }

    #[test]
    fn test_parse_quoted_message_without_bot_mention_is_ignored() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let msg = "> Quoted chat\n\nI disagree with your point.";
        let res = CommandParser::parse_message(msg, None, &bot_id, "!room:matrix.org", "alice");
        assert!(res.is_none());
    }

    #[test]
    fn test_parse_or_dm_prompt_command_in_dm_stays_a_command() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let ctx = CommandParser::parse_or_dm_prompt(
            "!todo add Buy milk",
            None,
            &bot_id,
            "!room:matrix.org",
            "alice",
            true,
        )
        .unwrap();

        assert_eq!(ctx.command, "todo");
        assert_eq!(ctx.raw_args, "add Buy milk");
    }

    #[test]
    fn test_parse_or_dm_prompt_plain_text_in_dm_becomes_ai_prompt() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let ctx = CommandParser::parse_or_dm_prompt(
            "What's the weather like tomorrow?",
            None,
            &bot_id,
            "!room:matrix.org",
            "alice",
            true,
        )
        .unwrap();

        assert_eq!(ctx.command, "ai");
        assert_eq!(ctx.raw_args, "What's the weather like tomorrow?");
        assert_eq!(ctx.full_body, "What's the weather like tomorrow?");
        assert!(!ctx.is_mentioned);
        assert!(ctx.quoted.is_none());
    }

    #[test]
    fn test_parse_or_dm_prompt_plain_text_outside_dm_is_ignored() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let res = CommandParser::parse_or_dm_prompt(
            "Just a random chatter message without bot mention",
            None,
            &bot_id,
            "!room:matrix.org",
            "charlie",
            false,
        );
        assert!(res.is_none());
    }

    #[test]
    fn test_parse_or_dm_prompt_whitespace_only_in_dm_is_ignored() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let res = CommandParser::parse_or_dm_prompt(
            "   \n\t  ",
            None,
            &bot_id,
            "!room:matrix.org",
            "alice",
            true,
        );
        assert!(res.is_none());
    }
}
