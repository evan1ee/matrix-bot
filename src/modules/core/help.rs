use crate::modules::CommandHelp;
use crate::utils::html_escape;

pub fn generate_help_html(module_helps: &[(String, Vec<CommandHelp>)]) -> String {
    let mut html = String::from("🤖 <b>Matrix Hello Bot Commands</b><br/><br/>");

    for (module_name, commands) in module_helps {
        if commands.is_empty() {
            continue;
        }
        html.push_str(&format!("📌 <b>{} Module:</b><br/>", html_escape(module_name)));
        for cmd in commands {
            html.push_str(&format!(
                "• <code>{}</code> - {}<br/>",
                html_escape(&cmd.usage),
                html_escape(&cmd.description)
            ));
        }
        html.push_str("<br/>");
    }

    html.push_str("<i>Tip: You can use <code>!command</code> or mention me <code>@bot command</code>.</i>");
    html
}
