use crate::interface::chat::CommandHelp;
use crate::utils::html_escape;

pub fn generate_help_html(module_helps: &[(String, Vec<CommandHelp>)]) -> String {
    let mut html = String::from("🤖 <b>Matrix Bot Commands</b><br/><br/>");

    for (module_name, commands) in module_helps {
        if commands.is_empty() {
            continue;
        }
        html.push_str(&format!(
            "📌 <b>{} Module:</b><br/>",
            html_escape(module_name)
        ));
        for cmd in commands {
            html.push_str(&format!(
                "• <code>{}</code> - {}<br/>",
                html_escape(&cmd.usage),
                html_escape(&cmd.description)
            ));
        }
        html.push_str("<br/>");
    }

    html.push_str(
        "<i>Tip: You can use <code>!command</code> or mention me <code>@bot command</code>.</i>",
    );
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_help_html() {
        let helps = vec![(
            "TestModule".to_string(),
            vec![CommandHelp {
                name: "test".to_string(),
                usage: "!test".to_string(),
                description: "Test description".to_string(),
            }],
        )];

        let html = generate_help_html(&helps);
        assert!(html.contains("TestModule Module:"));
        assert!(html.contains("<code>!test</code> - Test description"));
    }
}
