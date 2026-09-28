use tracing::error;

use crate::application::finance::FinanceService;
use crate::domain::accounts::AccountId;
use crate::utils::html_escape;

pub async fn handle_category(
    finance: &FinanceService,
    owner_id: &AccountId,
    rest: &str,
) -> Option<String> {
    let trimmed = rest.trim();
    let lower = trimmed.to_lowercase();

    if lower.is_empty() || lower == "list" {
        return match finance.list_categories(owner_id).await {
            Ok(cats) if cats.is_empty() => Some(
                "🏷️ No categories defined yet. Use <code>!category add &lt;name&gt;</code> to create one."
                    .to_string(),
            ),
            Ok(cats) => {
                let items = cats
                    .into_iter()
                    .map(|c| format!("• <b>{}</b>", html_escape(&c.name)))
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(format!("🏷️ <b>Categories:</b>\n{items}"))
            }
            Err(err) => {
                error!("Failed to list categories: {:?}", err);
                Some("❌ Database error fetching categories.".to_string())
            }
        };
    }

    if let Some(name) = lower.strip_prefix("add ") {
        let name = trimmed[trimmed.len() - name.len()..].trim();
        return match finance.create_category(owner_id, name).await {
            Ok(cat) => Some(format!(
                "🏷️ Added category <b>{}</b>.",
                html_escape(&cat.name)
            )),
            Err(crate::application::AppError::Validation(msg)) => Some(format!("⚠️ {msg}.")),
            Err(crate::application::AppError::Conflict(_)) => Some(format!(
                "⚠️ You already have a category named <b>{}</b>.",
                html_escape(name)
            )),
            Err(err) => {
                error!("Failed to create category: {:?}", err);
                Some("❌ Database error adding the category.".to_string())
            }
        };
    }

    if let Some(name) = lower
        .strip_prefix("remove ")
        .or_else(|| lower.strip_prefix("delete "))
    {
        let name = trimmed[trimmed.len() - name.len()..].trim();
        let cats = match finance.list_categories(owner_id).await {
            Ok(c) => c,
            Err(err) => {
                error!("Failed to list categories: {:?}", err);
                return Some("❌ Database error removing the category.".to_string());
            }
        };

        if let Some(cat) = cats.into_iter().find(|c| c.name.eq_ignore_ascii_case(name)) {
            return match finance.delete_category(owner_id, cat.id).await {
                Ok(Some(orphaned)) => Some(format!(
                    "🏷️ Removed <b>{}</b>. {} entr{} kept and left uncategorised.",
                    html_escape(&cat.name),
                    orphaned,
                    if orphaned == 1 { "y" } else { "ies" }
                )),
                Ok(None) => Some("⚠️ Category not found.".to_string()),
                Err(err) => {
                    error!("Failed to delete category: {:?}", err);
                    Some("❌ Database error removing the category.".to_string())
                }
            };
        } else {
            return Some(format!(
                "⚠️ No category named <b>{}</b>.",
                html_escape(name)
            ));
        }
    }

    Some("⚠️ Usage: <code>!category [add|list|remove] &lt;name&gt;</code>".to_string())
}
