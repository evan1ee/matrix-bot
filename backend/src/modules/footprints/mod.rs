use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod handler;
pub mod models;
pub mod repository;

pub struct FootprintsModule;

impl FootprintsModule {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FootprintsModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BotModule for FootprintsModule {
    fn name(&self) -> &str {
        "Footprints"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "footprint list".to_string(),
                usage: "!footprint list".to_string(),
                description: "List visited places and journey summary".to_string(),
            },
            CommandHelp {
                name: "footprint add".to_string(),
                usage: "!footprint add <name> [| YYYY-MM-DD] [| 1-5] [| note]".to_string(),
                description: "Record a newly visited place".to_string(),
            },
            CommandHelp {
                name: "footprint delete".to_string(),
                usage: "!footprint delete <id>".to_string(),
                description: "Delete a footprint entry by ID".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_footprint_command(
            &db.pool,
            broadcaster,
            &ctx.room_id,
            &ctx.owner_id,
            &ctx.sender_name,
            &ctx.command,
            &ctx.raw_args,
            &ctx.full_body,
            ctx.is_direct,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::modules::footprints::repository::FootprintRepository;
    use crate::db::Database;

    async fn setup_db() -> (Database, String) {
        let db = Database::connect("sqlite::memory:", 1).await.unwrap();
        crate::db::schema::run_migrations(&db.pool).await.unwrap();
        let account_id = crate::accounts::AccountRepository::resolve_or_provision(&db.pool, "@traveler:mbm.one")
            .await
            .unwrap();
        (db, account_id)
    }

    #[tokio::test]
    async fn test_footprint_crud_flow() {
        let (db, owner) = setup_db().await;

        let created = FootprintRepository::create(
            &db.pool,
            &owner,
            "Eiffel Tower",
            48.8584,
            2.2945,
            Some("France"),
            Some("Paris"),
            Some("Champ de Mars, 5 Av. Anatole France"),
            "2026-06-01",
            Some("2026-06-05"),
            45000,
            "EUR",
            2,
            5,
            "sightseeing",
            Some("Breathtaking night light show!"),
        )
        .await
        .unwrap();

        assert_eq!(created.title, "Eiffel Tower");
        assert_eq!(created.stay_duration_days, 4);
        assert_eq!(created.rating, 5);

        let list = FootprintRepository::list_by_owner(&db.pool, &owner, None, None, None)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);

        let stats = FootprintRepository::summary_stats(&db.pool, &owner).await.unwrap();
        assert_eq!(stats.total_places, 1);
        assert_eq!(stats.total_countries, 1);
        assert_eq!(stats.total_days_traveled, 4);

        let deleted = FootprintRepository::delete(&db.pool, created.id, &owner).await.unwrap();
        assert!(deleted);
    }
}
