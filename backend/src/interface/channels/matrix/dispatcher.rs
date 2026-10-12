use matrix_sdk::ruma::UserId;

use crate::domain::channels::{ConversationAddress, PlatformIdentity};
use crate::interface::chat::ParsedCommand;

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
        sender: &PlatformIdentity,
    ) -> Option<ParsedCommand> {
        let (quoted, prompt) = crate::interface::channels::matrix::quote::extract_quote_and_prompt(
            body,
            formatted_body,
        );
        let prompt_trimmed = prompt.trim();
        if prompt_trimmed.is_empty() && quoted.is_none() {
            return None;
        }

        let localpart = own_id.localpart();
        let full_mxid = own_id.to_string();
        let short_mention = format!("@{}", localpart);

        let is_html_mentioned = formatted_body
            .map(strip_mx_reply)
            .map(|f| f.contains(&full_mxid) || contains_word_mention_ci(&f, &short_mention))
            .unwrap_or(false);

        let is_plain_mentioned = prompt_trimmed.contains(&full_mxid)
            || contains_word_mention_ci(prompt_trimmed, &short_mention)
            || strip_leading_localpart_mention(prompt_trimmed, localpart).is_some();

        let is_mentioned = is_plain_mentioned || is_html_mentioned;
        let is_command_prefix = prompt_trimmed.starts_with('!') || prompt_trimmed.starts_with('/');

        if !is_command_prefix && !is_mentioned {
            return None;
        }

        let without_mxid = prompt_trimmed.replace(&full_mxid, "");
        let without_short = remove_word_mentions_ci(&without_mxid, &short_mention);
        let mut clean_body = without_short.trim().to_string();

        if let Some(after_localpart) = strip_leading_localpart_mention(&clean_body, localpart) {
            clean_body = after_localpart.trim().to_string();
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
                .trim_start_matches(['!', '/'])
                .to_string();
            let args = parts.next().unwrap_or("").trim().to_string();
            (cmd, args)
        };

        Some(ParsedCommand {
            conversation: ConversationAddress::Matrix(room_id.to_string()),
            sender: sender.clone(),
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
        sender: &PlatformIdentity,
        is_direct: bool,
    ) -> Option<ParsedCommand> {
        if let Some(parsed) = Self::parse_message(body, formatted_body, own_id, room_id, sender) {
            return Some(parsed);
        }

        let trimmed = body.trim();
        if is_direct && !trimmed.is_empty() {
            let (quoted, prompt) =
                crate::interface::channels::matrix::quote::extract_quote_and_prompt(
                    trimmed,
                    formatted_body,
                );
            Some(ParsedCommand {
                conversation: ConversationAddress::Matrix(room_id.to_string()),
                sender: sender.clone(),
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
                "note",
                "notes",
                "search",
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

fn strip_mx_reply(formatted: &str) -> String {
    if let Some(end_idx) = formatted.find("</mx-reply>") {
        formatted[end_idx + "</mx-reply>".len()..].to_string()
    } else {
        formatted.to_string()
    }
}

fn is_mention_boundary_after(c: Option<char>) -> bool {
    match c {
        None => true,
        Some(ch) => !ch.is_alphanumeric() && ch != '_' && ch != '-',
    }
}

fn is_mention_boundary_before(c: Option<char>) -> bool {
    match c {
        None => true,
        Some(ch) => !ch.is_alphanumeric() && ch != '_' && ch != '-',
    }
}

fn match_ci_prefix<'a>(haystack: &'a str, needle: &str) -> Option<&'a str> {
    let mut h_chars = haystack.char_indices();
    for nc in needle.chars() {
        let (_, hc) = h_chars.next()?;
        if !hc.to_lowercase().eq(nc.to_lowercase()) {
            return None;
        }
    }
    let next_byte = h_chars.next().map(|(i, _)| i).unwrap_or(haystack.len());
    Some(&haystack[next_byte..])
}

/// Checks if `haystack` starts with `localpart` followed by `:`, `,`, or end-of-string.
fn strip_leading_localpart_mention<'a>(haystack: &'a str, localpart: &str) -> Option<&'a str> {
    let rest = match_ci_prefix(haystack, localpart)?;
    if rest.is_empty() {
        Some(rest)
    } else if let Some(after) = rest.strip_prefix(':').or_else(|| rest.strip_prefix(',')) {
        Some(after)
    } else {
        None
    }
}

/// Checks whether `mention` (e.g. `@bot`) appears in `haystack` on word boundaries.
fn contains_word_mention_ci(haystack: &str, mention: &str) -> bool {
    let mut prev_char = None;
    for (idx, ch) in haystack.char_indices() {
        if is_mention_boundary_before(prev_char) {
            if let Some(rest) = match_ci_prefix(&haystack[idx..], mention) {
                if is_mention_boundary_after(rest.chars().next()) {
                    return true;
                }
            }
        }
        prev_char = Some(ch);
    }
    false
}

/// Removes `mention` (e.g. `@bot`) from `haystack` only where it occurs on word boundaries.
fn remove_word_mentions_ci(haystack: &str, mention: &str) -> String {
    let mut out = String::with_capacity(haystack.len());
    let mut idx = 0;
    let mut prev_char = None;
    while idx < haystack.len() {
        if is_mention_boundary_before(prev_char) {
            if let Some(rest) = match_ci_prefix(&haystack[idx..], mention) {
                if is_mention_boundary_after(rest.chars().next()) {
                    idx = haystack.len() - rest.len();
                    prev_char = None;
                    continue;
                }
            }
        }
        let ch = haystack[idx..].chars().next().unwrap();
        out.push(ch);
        idx += ch.len_utf8();
        prev_char = Some(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> PlatformIdentity {
        PlatformIdentity::matrix("@alice:matrix.org").unwrap()
    }

    #[test]
    fn test_parse_message_exclamation_command() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let ctx = CommandParser::parse_message(
            "!todo add Buy milk",
            None,
            &bot_id,
            "!room:matrix.org",
            &PlatformIdentity::matrix("@alice:matrix.org").unwrap(),
        )
        .unwrap();

        assert_eq!(ctx.command, "todo");
        assert_eq!(ctx.raw_args, "add Buy milk");
        assert_eq!(ctx.sender_name(), "alice");
    }

    #[test]
    fn test_parse_message_slash_command() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let ctx = CommandParser::parse_message(
            "/note Deep Work Tactics #focus",
            None,
            &bot_id,
            "!room:matrix.org",
            &PlatformIdentity::matrix("@alice:matrix.org").unwrap(),
        )
        .unwrap();

        assert_eq!(ctx.command, "note");
        assert_eq!(ctx.raw_args, "Deep Work Tactics #focus");
        assert_eq!(ctx.sender_name(), "alice");
    }

    #[test]
    fn test_parse_message_mention_command() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let ctx = CommandParser::parse_message(
            "@hellobot remind 10s test",
            None,
            &bot_id,
            "!room:matrix.org",
            &PlatformIdentity::matrix("@bob:matrix.org").unwrap(),
        )
        .unwrap();

        assert_eq!(ctx.command, "remind");
        assert_eq!(ctx.raw_args, "10s test");
        assert_eq!(ctx.sender_name(), "bob");
    }

    #[test]
    fn test_parse_non_command_message() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let res = CommandParser::parse_message(
            "Just a random chatter message without bot mention",
            None,
            &bot_id,
            "!room:matrix.org",
            &PlatformIdentity::matrix("@charlie:matrix.org").unwrap(),
        );
        assert!(res.is_none());
    }

    #[test]
    fn test_parse_quoted_reply_fact_check() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let msg =
            "> <@bob:matrix.org> The moon is made of green cheese.\n\n@hellobot fact check this";
        let ctx =
            CommandParser::parse_message(msg, None, &bot_id, "!room:matrix.org", &alice()).unwrap();

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
            CommandParser::parse_message(msg, None, &bot_id, "!room:matrix.org", &alice()).unwrap();

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
            CommandParser::parse_message(msg, None, &bot_id, "!room:matrix.org", &alice()).unwrap();

        assert_eq!(ctx.command, "ai");
        assert!(ctx.quoted.is_some());
        assert_eq!(ctx.quoted.unwrap().body, "Important news snippet");
    }

    #[test]
    fn test_parse_quoted_message_without_bot_mention_is_ignored() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let msg = "> Quoted chat\n\nI disagree with your point.";
        let res = CommandParser::parse_message(msg, None, &bot_id, "!room:matrix.org", &alice());
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
            &PlatformIdentity::matrix("@alice:matrix.org").unwrap(),
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
            &PlatformIdentity::matrix("@alice:matrix.org").unwrap(),
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
            &PlatformIdentity::matrix("@charlie:matrix.org").unwrap(),
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
            &PlatformIdentity::matrix("@alice:matrix.org").unwrap(),
            true,
        );
        assert!(res.is_none());
    }

    #[test]
    fn test_mention_does_not_corrupt_words_sharing_localpart_prefix() {
        let bot_id = UserId::parse("@bot:matrix.org").unwrap();
        let ctx = CommandParser::parse_message(
            "@bot botanical garden",
            None,
            &bot_id,
            "!room:matrix.org",
            &alice(),
        )
        .unwrap();
        assert_eq!(ctx.command, "botanical");
        assert_eq!(ctx.raw_args, "garden");

        // Substring mention (@botanical) is not a mention of @bot
        assert!(CommandParser::parse_message(
            "Visit @botanical garden",
            None,
            &bot_id,
            "!room:matrix.org",
            &alice(),
        )
        .is_none());

        // Multibyte case-folding character before localpart must not panic
        let _ = CommandParser::parse_message(
            "İbot: hello",
            None,
            &bot_id,
            "!room:matrix.org",
            &alice(),
        );
    }

    #[test]
    fn test_html_mention_inside_mx_reply_only_is_ignored() {
        let bot_id = UserId::parse("@hellobot:matrix.org").unwrap();
        let body = "> <@hellobot:matrix.org> earlier reply\n\nI agree with that.";
        let formatted = "<mx-reply><blockquote><a href=\"https://matrix.to/#/@hellobot:matrix.org\">@hellobot:matrix.org</a> earlier reply</blockquote></mx-reply>I agree with that.";
        let res = CommandParser::parse_message(
            body,
            Some(formatted),
            &bot_id,
            "!room:matrix.org",
            &alice(),
        );
        assert!(res.is_none());
    }
}
