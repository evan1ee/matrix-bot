//! Chat front door, one file per feature: each feature's Matrix side — its
//! `BotModule` command handler, and any Matrix-specific lookups it needs —
//! calling that feature's application service. Holds no SQL
//! (`tests/architecture.rs`).

pub mod account;
pub mod bookmarks;
pub mod footprints;
pub mod habits;
pub mod recurring_bills;
pub mod reminders;
pub mod rooms;
pub mod settings;
pub mod todos;
pub mod webhooks;

use tracing::error;

use crate::application::AppError;
use crate::utils::html::html_escape;

/// What a failed use case tells the user in chat: the chat counterpart of
/// `interface/http/app_error.rs`, and the only place a command handler
/// turns an `AppError` into a reply.
///
/// Returns the HTML reply a `BotModule` hands back. The four user-facing
/// variants show their (escaped) message; a database or external failure is
/// logged here and answered with a fixed apology, so a SQL or provider error
/// never reaches a room.
pub fn error_reply(err: AppError) -> String {
    let message = match err {
        AppError::Validation(msg)
        | AppError::NotFound(msg)
        | AppError::Forbidden(msg)
        | AppError::Conflict(msg) => msg,
        AppError::Database(err) => {
            error!(error = %err, "Chat command failed on the database");
            return INTERNAL_REPLY.to_owned();
        }
        AppError::External { service, source } => {
            error!(service, error = %format!("{source:#}"), "Chat command failed on a dependency");
            return INTERNAL_REPLY.to_owned();
        }
    };
    format!("⚠️ {}", html_escape(&message))
}

const INTERNAL_REPLY: &str = "⚠️ Something went wrong on our side. Please try again later.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_facing_errors_show_their_escaped_message() {
        let reply = error_reply(AppError::Validation("<b>too long</b>".into()));
        assert_eq!(reply, "⚠️ &lt;b&gt;too long&lt;/b&gt;");
        assert_eq!(
            error_reply(AppError::NotFound("No such todo".into())),
            "⚠️ No such todo"
        );
    }

    #[test]
    fn diagnostic_errors_never_reach_the_room() {
        let db = error_reply(AppError::Database(sqlx::Error::Protocol(
            "relation \"account_rooms\" does not exist".into(),
        )));
        let ext = error_reply(AppError::External {
            service: "ocr",
            source: anyhow::anyhow!("10.0.0.5 refused"),
        });
        assert_eq!(db, INTERNAL_REPLY);
        assert_eq!(ext, INTERNAL_REPLY);
    }
}
