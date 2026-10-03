//! The slash commands Discord shows, and how one becomes the registry's input.
//!
//! Spec D6: every command takes a single string (`/todo args:"add milk"` is
//! `!todo add milk`), so the shared `BotModule` registry parses it unchanged.

use serenity::all::{CommandOptionType, CreateCommand, CreateCommandOption};

use crate::interface::channels::inbound::InboundText;

/// The chat input cap every command shares (`AGENTS.md` §4.2).
pub const ARGS_MAX: usize = 500;
/// `/ask`'s prompt cap: below `llm::turn::MAX_PROMPT_LENGTH`, within Discord's 6000.
pub const PROMPT_MAX: usize = 4000;

pub struct CommandSpec {
    /// The slash command.
    pub name: &'static str,
    /// The registry command it runs ("ai" for `/ask`).
    pub dispatches_as: &'static str,
    pub description: &'static str,
    /// The one string option: "args" or "prompt".
    pub option: &'static str,
    pub required: bool,
    pub max_length: usize,
}

const fn args(name: &'static str, description: &'static str) -> CommandSpec {
    CommandSpec {
        name,
        dispatches_as: name,
        description,
        option: "args",
        required: false,
        max_length: ARGS_MAX,
    }
}

/// `/todo /expense /portfolio /ask /link /help` (spec §8.2).
pub const COMMANDS: &[CommandSpec] = &[
    args("todo", "Your todo list: add, list, done"),
    args("expense", "Record and list expenses"),
    args("portfolio", "Your investment portfolio"),
    CommandSpec {
        name: "ask",
        dispatches_as: "ai",
        description: "Ask the assistant",
        option: "prompt",
        required: true,
        max_length: PROMPT_MAX,
    },
    args(
        "link",
        "Connect this Discord account to your workspace (DM only)",
    ),
    args("help", "What the bot can do"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeferMode {
    Public,
    Ephemeral,
}

/// B2: in a guild every command but `help` answers ephemerally (todos,
/// expenses, portfolio and agent answers are all personal); a DM is private
/// already, so nothing there is ephemeral.
pub fn defer_mode(command: &str, in_guild: bool) -> DeferMode {
    if in_guild && command != "help" {
        DeferMode::Ephemeral
    } else {
        DeferMode::Public
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CommandRefusal {
    Unknown(String),
    TooLong { max: usize },
    MissingPrompt,
}

impl CommandRefusal {
    /// Shown ephemerally; a refused command is never dispatched.
    pub fn reply_text(&self) -> String {
        match self {
            Self::Unknown(name) => format!("Unknown command /{name}."),
            Self::TooLong { max } => format!("That is too long (at most {max} characters)."),
            Self::MissingPrompt => "Tell me what to ask: /ask prompt:…".to_owned(),
        }
    }
}

/// The slash command and its one option value → the registry's input.
///
/// `full_body` is the bare form `todo add milk` (B1): modules parse it and
/// accept `!todo` or `todo`, never `/todo`.
pub fn inbound_text(name: &str, value: Option<&str>) -> Result<InboundText, CommandRefusal> {
    let spec = COMMANDS
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| CommandRefusal::Unknown(name.to_owned()))?;
    let value = value.map(str::trim).unwrap_or_default();
    // Discord's `max_length` is a client hint; a crafted interaction can exceed it.
    if value.chars().count() > spec.max_length {
        return Err(CommandRefusal::TooLong {
            max: spec.max_length,
        });
    }
    if spec.required && value.is_empty() {
        return Err(CommandRefusal::MissingPrompt);
    }
    let command = spec.dispatches_as.to_owned();
    let full_body = format!("{command} {value}").trim().to_owned();
    Ok(InboundText::Command {
        command,
        raw_args: value.to_owned(),
        full_body,
    })
}

/// serenity builders for registration; the table test on `COMMANDS` covers them.
pub fn create_commands() -> Vec<CreateCommand> {
    COMMANDS
        .iter()
        .map(|spec| {
            let max = u16::try_from(spec.max_length).unwrap_or(u16::MAX);
            CreateCommand::new(spec.name)
                .description(spec.description)
                .dm_permission(true)
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        spec.option,
                        if spec.required {
                            "What to ask"
                        } else {
                            "Arguments, as after the command on Matrix"
                        },
                    )
                    .required(spec.required)
                    .max_length(max),
                )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(text: InboundText) -> (String, String, String) {
        match text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => (command, raw_args, full_body),
            InboundText::FreeText(t) => panic!("expected a command, got free text {t}"),
        }
    }

    #[test]
    fn the_table_matches_the_spec() {
        let names: Vec<_> = COMMANDS.iter().map(|c| c.name).collect();
        assert_eq!(
            names,
            ["todo", "expense", "portfolio", "ask", "link", "help"]
        );
        for spec in COMMANDS {
            if spec.name == "ask" {
                assert_eq!(
                    (
                        spec.dispatches_as,
                        spec.option,
                        spec.required,
                        spec.max_length
                    ),
                    ("ai", "prompt", true, PROMPT_MAX)
                );
            } else {
                assert_eq!(
                    (
                        spec.dispatches_as,
                        spec.option,
                        spec.required,
                        spec.max_length
                    ),
                    (spec.name, "args", false, ARGS_MAX)
                );
            }
        }
        assert_eq!(create_commands().len(), COMMANDS.len());
    }

    #[test]
    fn registration_builders_carry_the_option_limits() {
        let json = serde_json::to_value(create_commands()).unwrap();
        let ask = json
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "ask")
            .unwrap();
        assert_eq!(ask["options"][0]["name"], "prompt");
        assert_eq!(ask["options"][0]["required"], true);
        assert_eq!(ask["options"][0]["max_length"], 4000);
        assert_eq!(ask["dm_permission"], true);
    }

    #[test]
    fn defer_mode_is_ephemeral_in_guilds_except_help() {
        for spec in COMMANDS {
            assert_eq!(
                defer_mode(spec.name, false),
                DeferMode::Public,
                "{}",
                spec.name
            );
            let expected = if spec.name == "help" {
                DeferMode::Public
            } else {
                DeferMode::Ephemeral
            };
            assert_eq!(defer_mode(spec.name, true), expected, "{}", spec.name);
        }
    }

    #[test]
    fn inbound_text_builds_the_bare_full_body() {
        assert_eq!(
            command(inbound_text("todo", Some(" add milk ")).unwrap()),
            ("todo".into(), "add milk".into(), "todo add milk".into())
        );
        assert_eq!(
            command(inbound_text("todo", None).unwrap()),
            ("todo".into(), String::new(), "todo".into())
        );
        assert_eq!(
            command(inbound_text("ask", Some("hi")).unwrap()),
            ("ai".into(), "hi".into(), "ai hi".into())
        );
    }

    #[test]
    fn over_long_and_missing_values_are_refused() {
        let long = "x".repeat(ARGS_MAX + 1);
        assert_eq!(
            inbound_text("todo", Some(&long)).err(),
            Some(CommandRefusal::TooLong { max: ARGS_MAX })
        );
        assert!(inbound_text("todo", Some(&"字".repeat(ARGS_MAX))).is_ok());
        let prompt = "x".repeat(PROMPT_MAX + 1);
        assert_eq!(
            inbound_text("ask", Some(&prompt)).err(),
            Some(CommandRefusal::TooLong { max: PROMPT_MAX })
        );
        assert_eq!(
            inbound_text("ask", Some("  ")).err(),
            Some(CommandRefusal::MissingPrompt)
        );
        assert_eq!(
            inbound_text("rm", None).err(),
            Some(CommandRefusal::Unknown("rm".into()))
        );
    }
}
