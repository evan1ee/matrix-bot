use crate::domain::channels::Channel;
use crate::interface::chat::CommandHelp;
use crate::utils::html_escape;

pub fn generate_help_html(module_helps: &[(String, Vec<CommandHelp>)], channel: Channel) -> String {
    let (header, tip, slash_style) = match channel {
        Channel::Discord => (
            "🤖 <b>Discord Bot Commands</b><br/><br/>",
            "<i>Tip: You can use <code>/command</code> or mention me <code>@bot command</code>.</i>",
            true,
        ),
        Channel::Telegram => (
            "🤖 <b>Telegram Bot Commands</b><br/><br/>",
            "<i>Tip: You can use <code>/command</code> or send a message directly.</i>",
            true,
        ),
        Channel::Email => (
            "🤖 <b>Email Bot Commands</b><br/><br/>",
            "<i>Tip: Reply directly or include <code>command &lt;args&gt;</code> in subject/body.</i>",
            false,
        ),
        Channel::Matrix => (
            "🤖 <b>Matrix Bot Commands</b><br/><br/>",
            "<i>Tip: You can use <code>!command</code> or mention me <code>@bot command</code>.</i>",
            false,
        ),
    };

    let mut html = String::from(header);

    for (module_name, commands) in module_helps {
        if commands.is_empty() {
            continue;
        }
        html.push_str(&format!(
            "📌 <b>{} Module:</b><br/>",
            html_escape(module_name)
        ));
        for cmd in commands {
            let usage = if slash_style {
                cmd.usage.replace("! ", "/").replace('!', "/")
            } else {
                cmd.usage.clone()
            };
            html.push_str(&format!(
                "• <code>{}</code> - {}<br/>",
                html_escape(&usage),
                html_escape(&cmd.description)
            ));
        }
        html.push_str("<br/>");
    }

    html.push_str(tip);
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_help_html_matrix() {
        let helps = vec![(
            "TestModule".to_string(),
            vec![CommandHelp {
                name: "test".to_string(),
                usage: "!test".to_string(),
                description: "Test description".to_string(),
            }],
        )];

        let html = generate_help_html(&helps, Channel::Matrix);
        assert!(html.contains("Matrix Bot Commands"));
        assert!(html.contains("TestModule Module:"));
        assert!(html.contains("<code>!test</code> - Test description"));
        assert!(html.contains("<code>!command</code>"));
    }

    #[test]
    fn test_generate_help_html_discord_and_telegram() {
        let helps = vec![(
            "Tasks".to_string(),
            vec![CommandHelp {
                name: "todo".to_string(),
                usage: "!todo list".to_string(),
                description: "List tasks".to_string(),
            }],
        )];

        let discord_html = generate_help_html(&helps, Channel::Discord);
        assert!(discord_html.contains("Discord Bot Commands"));
        assert!(discord_html.contains("<code>/todo list</code> - List tasks"));
        assert!(discord_html.contains("<code>/command</code>"));

        let telegram_html = generate_help_html(&helps, Channel::Telegram);
        assert!(telegram_html.contains("Telegram Bot Commands"));
        assert!(telegram_html.contains("<code>/todo list</code> - List tasks"));
    }
}
