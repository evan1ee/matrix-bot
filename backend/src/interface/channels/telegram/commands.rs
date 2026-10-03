//! `/name[@bot] args` parsing and the commands Telegram's menu advertises.
//!
//! T2: any `/name` is parsed as a command (the registry ignores names no
//! module claims), so every module is reachable by typing; the menu lists only
//! the common ones. `full_body` is the bare form (`todo add milk`, R85).

use crate::interface::channels::inbound::InboundText;

/// The chat input cap every command shares (`AGENTS.md` §4.2).
pub const ARGS_MAX: usize = 500;

/// `setMyCommands` entries: `(command, description)`; the description is 3-256
/// characters in Telegram's rules.
pub const ADVERTISED: &[(&str, &str)] = &[
    ("start", "Say hello and see what the bot can do"),
    ("help", "What the bot can do"),
    ("todo", "Your todo list: add, list, done"),
    ("expense", "Record and list expenses"),
    ("portfolio", "Your investment portfolio"),
    (
        "link",
        "Connect this Telegram account to your workspace (private chat only)",
    ),
];

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Command {
        command: String,
        args: String,
    },
    /// `/cmd@OtherBot`: addressed to a different bot sharing the group.
    NotForUs,
    NotACommand,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CommandRefusal {
    TooLong { max: usize },
}

impl CommandRefusal {
    /// What the user is told; a refused command is never dispatched.
    pub fn reply_text(&self) -> String {
        match self {
            Self::TooLong { max } => format!("That is too long (at most {max} characters)."),
        }
    }
}

/// `/name[@bot] args` → Command (name lower-cased; `@bot` must equal
/// `our_username`, case-insensitive, else `NotForUs`). Anything not starting
/// with `/`, a bare `/` and `/@bot` are `NotACommand`.
pub fn parse_command(text: &str, our_username: &str) -> Parsed {
    let Some(rest) = text.trim_start().strip_prefix('/') else {
        return Parsed::NotACommand;
    };
    let (head, args) = split_head(rest);
    let (name, target) = match head.split_once('@') {
        Some((name, target)) => (name, Some(target)),
        None => (head, None),
    };
    if name.is_empty() {
        return Parsed::NotACommand;
    }
    if target.is_some_and(|t| !t.eq_ignore_ascii_case(our_username)) {
        return Parsed::NotForUs;
    }
    command(name, args)
}

/// Callback data (no leading `/`, T6) → Command or NotACommand. The data is
/// untrusted client input: it is parsed exactly like typed text.
pub fn parse_callback(data: &str) -> Parsed {
    let data = data.trim_start();
    if data.starts_with('/') {
        return Parsed::NotACommand;
    }
    let (name, args) = split_head(data);
    if name.is_empty() {
        return Parsed::NotACommand;
    }
    command(name, args)
}

/// T2: Command → InboundText with the bare `full_body`; args over `ARGS_MAX`
/// characters are refused.
pub fn inbound_text(command: &str, args: &str) -> Result<InboundText, CommandRefusal> {
    let args = args.trim();
    if args.chars().count() > ARGS_MAX {
        return Err(CommandRefusal::TooLong { max: ARGS_MAX });
    }
    Ok(InboundText::Command {
        command: command.to_owned(),
        raw_args: args.to_owned(),
        full_body: format!("{command} {args}").trim().to_owned(),
    })
}

fn command(name: &str, args: &str) -> Parsed {
    Parsed::Command {
        command: name.to_lowercase(),
        args: args.to_owned(),
    }
}

/// The first whitespace-delimited word and the trimmed remainder.
fn split_head(s: &str) -> (&str, &str) {
    match s.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim()),
        None => (s, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(command: &str, args: &str) -> Parsed {
        Parsed::Command {
            command: command.to_owned(),
            args: args.to_owned(),
        }
    }

    #[test]
    fn a_plain_command_splits_name_and_args() {
        assert_eq!(
            parse_command("/todo add milk", "MyBot"),
            cmd("todo", "add milk")
        );
        assert_eq!(parse_command("/help", "MyBot"), cmd("help", ""));
    }

    #[test]
    fn a_suffix_for_us_is_accepted_case_insensitively() {
        assert_eq!(
            parse_command("/todo@MyBot add milk", "MyBot"),
            cmd("todo", "add milk")
        );
        assert_eq!(
            parse_command("/todo@mybot add milk", "MyBot"),
            cmd("todo", "add milk")
        );
    }

    #[test]
    fn a_suffix_for_another_bot_is_not_for_us() {
        assert_eq!(
            parse_command("/todo@OtherBot add milk", "MyBot"),
            Parsed::NotForUs
        );
    }

    #[test]
    fn the_name_is_lowercased_and_args_keep_their_case() {
        assert_eq!(
            parse_command("/Todo Add Milk", "MyBot"),
            cmd("todo", "Add Milk")
        );
    }

    #[test]
    fn text_without_a_slash_and_bare_slashes_are_not_commands() {
        assert_eq!(parse_command("add milk", "MyBot"), Parsed::NotACommand);
        assert_eq!(parse_command("/", "MyBot"), Parsed::NotACommand);
        assert_eq!(parse_command("/@MyBot", "MyBot"), Parsed::NotACommand);
        assert_eq!(parse_command("/ todo", "MyBot"), Parsed::NotACommand);
    }

    #[test]
    fn a_newline_ends_the_name_like_a_space() {
        assert_eq!(
            parse_command("/todo\nadd milk", "MyBot"),
            cmd("todo", "add milk")
        );
    }

    #[test]
    fn callback_data_is_a_command_without_a_slash() {
        assert_eq!(parse_callback("todo done 12"), cmd("todo", "done 12"));
        assert_eq!(parse_callback(""), Parsed::NotACommand);
        assert_eq!(parse_callback("/todo done 12"), Parsed::NotACommand);
    }

    #[test]
    fn the_inbound_text_is_the_bare_form() {
        match inbound_text("todo", "add milk").unwrap() {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "add milk");
                assert_eq!(full_body, "todo add milk");
            }
            InboundText::FreeText(_) => panic!("expected a command"),
        }
        match inbound_text("help", "").unwrap() {
            InboundText::Command { full_body, .. } => assert_eq!(full_body, "help"),
            InboundText::FreeText(_) => panic!("expected a command"),
        }
    }

    #[test]
    fn over_long_arguments_are_refused() {
        let refused = inbound_text("todo", &"x".repeat(501)).err();
        assert_eq!(refused, Some(CommandRefusal::TooLong { max: 500 }));
        assert!(refused.unwrap().reply_text().contains("500"));
        assert!(inbound_text("todo", &"x".repeat(500)).is_ok());
    }

    #[test]
    fn the_advertised_commands_are_the_documented_set() {
        let names: Vec<&str> = ADVERTISED.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            ["start", "help", "todo", "expense", "portfolio", "link"]
        );
        assert!(ADVERTISED
            .iter()
            .all(|(_, d)| !d.is_empty() && d.len() <= 256));
    }
}
