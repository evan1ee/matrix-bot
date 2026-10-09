//! AI and MCP tool definitions and executors for the notes knowledge base.

use serde_json::{json, Value};

use crate::application::knowledge::SearchQuery;
use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::channels::ConversationAddress;
use crate::domain::knowledge::note::{NewNote, NoteDocument, NoteType};
use crate::infrastructure::inference::llm::ToolDefinition;

/// Longest note body `knowledge_get_note` returns, in characters. A note can
/// hold 100k characters; the model gets the start and is told it was cut.
const GET_NOTE_MAX_CHARS: usize = 8_000;

pub async fn execute_tool(
    services: &AppServices,
    owner_id: &AccountId,
    room_id: &str,
    name: &str,
    args: &Value,
) -> Option<Result<String, String>> {
    let result = match name {
        "knowledge_search" => search(services, owner_id, args).await,
        "knowledge_create_note" => create_note(services, owner_id, room_id, args).await,
        "knowledge_get_note" => get_note(services, owner_id, args).await,
        "knowledge_list_tags" => list_tags(services, owner_id).await,
        _ => return None,
    };
    Some(result)
}

/// The `notes.source_channel` value for a tool call, taken from the
/// conversation it runs in. The column only accepts the five known sources.
fn source_channel(room_id: &str) -> &'static str {
    if room_id == "mcp" {
        return "mcp";
    }
    ConversationAddress::parse_storage(room_id)
        .ok()
        .and_then(|address| address.channel())
        .map_or("web", |channel| channel.as_str())
}

fn string_list(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key).and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect()
    })
}

async fn search(
    services: &AppServices,
    owner_id: &AccountId,
    args: &Value,
) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .ok_or("Field 'query' is required")?;
    let filter_types = string_list(args, "filter_types");
    let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(5);

    let outcome = services
        .knowledge_search
        .search(
            owner_id,
            SearchQuery::new(query)
                .filter_types(filter_types.as_deref())
                .limit(limit),
        )
        .await
        .map_err(|e| e.to_string())?;

    // `mode` tells the model whether semantic ranking took part.
    serde_json::to_string(&json!({
        "mode": outcome.mode.as_str(),
        "results": outcome.items,
    }))
    .map_err(|e| e.to_string())
}

async fn create_note(
    services: &AppServices,
    owner_id: &AccountId,
    room_id: &str,
    args: &Value,
) -> Result<String, String> {
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .ok_or("Field 'title' is required")?;
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or("Field 'content' is required")?;
    let note_type = match args.get("note_type").and_then(Value::as_str) {
        Some(raw) => raw.parse::<NoteType>().map_err(|e| e.to_string())?,
        None => NoteType::default(),
    };

    let note = services
        .notes
        .create(
            owner_id,
            NewNote::new(title, content)
                .note_type(note_type)
                .source(source_channel(room_id), None)
                .tags(string_list(args, "tags").unwrap_or_default()),
        )
        .await
        .map_err(|e| e.to_string())?;

    serde_json::to_string(&note).map_err(|e| e.to_string())
}

/// Reads one note for the turn's owner. The id is the model's, the owner
/// never is: another account's id is indistinguishable from a missing one.
async fn get_note(
    services: &AppServices,
    owner_id: &AccountId,
    args: &Value,
) -> Result<String, String> {
    let id = args
        .get("id")
        .and_then(Value::as_i64)
        .ok_or("Field 'id' is required and must be an integer")?;
    let document = services
        .notes
        .get(owner_id, id)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_string(&note_view(&document)).map_err(|e| e.to_string())
}

fn note_view(document: &NoteDocument) -> Value {
    let total_chars = document.markdown.chars().count();
    let truncated = total_chars > GET_NOTE_MAX_CHARS;
    let content: String = document.markdown.chars().take(GET_NOTE_MAX_CHARS).collect();
    json!({
        "id": document.note.id,
        "title": document.note.title,
        "note_type": document.note.note_type,
        "tags": document.note.tags,
        "content": content,
        "truncated": truncated,
        "total_chars": total_chars,
    })
}

async fn list_tags(services: &AppServices, owner_id: &AccountId) -> Result<String, String> {
    let tags = services
        .notes
        .list_tags(owner_id)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_string(&tags).map_err(|e| e.to_string())
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "knowledge_search",
            "Perform hybrid vector semantic and keyword search across your notes, bookmarks, journal entries and footprints.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language query, question, or keyword to search for (required)"
                    },
                    "filter_types": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": ["note", "bookmark", "journal", "footprint"]
                        },
                        "description": "Optional list of item types to filter search results"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of ranked results to return (default 5)"
                    }
                },
                "required": ["query"]
            }),
        ),
        ToolDefinition::function(
            "knowledge_create_note",
            "Save an idea, thought or document as a note.",
            json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Title of the note (required, 1-100 characters)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Note content in Markdown format (required)"
                    },
                    "note_type": {
                        "type": "string",
                        "enum": ["note", "idea", "fact", "observation", "decision"],
                        "description": "What kind of note this is (default 'note')"
                    },
                    "tags": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "description": "Optional tags to categorize this note"
                    }
                },
                "required": ["title", "content"]
            }),
        ),
        ToolDefinition::function(
            "knowledge_get_note",
            "Read one of your notes in full by id, as returned by knowledge_search or knowledge_create_note. Long notes are cut and flagged.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "Id of the note (required)"
                    }
                },
                "required": ["id"]
            }),
        ),
        ToolDefinition::function(
            "knowledge_list_tags",
            "List all unique tags used across notes in the knowledge base.",
            json!({
                "type": "object",
                "properties": {}
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::knowledge::note::NoteType;

    async fn setup(seed: &str) -> (AppServices, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        (AppServices::new(&pool), owner)
    }

    async fn run(
        services: &AppServices,
        owner: &AccountId,
        room_id: &str,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let out = execute_tool(services, owner, room_id, name, &args)
            .await
            .expect("tool is handled by knowledge::execute_tool")?;
        Ok(serde_json::from_str(&out).expect("tool output is JSON"))
    }

    #[test]
    fn the_note_source_follows_the_conversation_the_tool_ran_in() {
        assert_eq!(source_channel("mcp"), "mcp");
        assert_eq!(source_channel("!room:mbm.one"), "matrix");
        assert_eq!(source_channel("web"), "web");
        assert_eq!(source_channel("web_chat"), "web");
        assert_eq!(source_channel("telegram:42"), "telegram");
        assert_eq!(source_channel(""), "web");
    }

    #[test]
    fn the_registry_describes_get_note_and_the_note_type_enum() {
        let defs = tool_definitions();
        let get = defs
            .iter()
            .find(|d| d.function.name == "knowledge_get_note")
            .expect("knowledge_get_note is registered");
        assert_eq!(get.function.parameters["required"], json!(["id"]));
        assert!(get.function.parameters["properties"]
            .get("owner_id")
            .is_none());

        let create = defs
            .iter()
            .find(|d| d.function.name == "knowledge_create_note")
            .unwrap();
        assert_eq!(
            create.function.parameters["properties"]["note_type"]["enum"],
            json!(["note", "idea", "fact", "observation", "decision"])
        );
        assert_eq!(
            create.function.parameters["required"],
            json!(["title", "content"])
        );
    }

    #[tokio::test]
    async fn create_note_stores_the_type_and_a_source_the_schema_accepts() {
        let (services, owner) = setup("kt_create").await;

        let created = run(
            &services,
            &owner,
            "mcp",
            "knowledge_create_note",
            json!({"title": "Pick Postgres", "content": "# Why
Boring tech.", "note_type": "decision"}),
        )
        .await
        .unwrap();

        let id = created["id"].as_i64().unwrap();
        let stored = services.notes.get(&owner, id).await.unwrap();
        assert_eq!(stored.note.note_type, NoteType::Decision);
        assert_eq!(stored.note.source_channel, "mcp");
        assert_eq!(
            stored.markdown,
            "# Why
Boring tech."
        );
    }

    #[tokio::test]
    async fn create_note_defaults_to_a_plain_note_and_rejects_an_unknown_type() {
        let (services, owner) = setup("kt_create_type").await;

        let plain = run(
            &services,
            &owner,
            "!room:mbm.one",
            "knowledge_create_note",
            json!({"title": "Plain", "content": "body"}),
        )
        .await
        .unwrap();
        let stored = services
            .notes
            .get(&owner, plain["id"].as_i64().unwrap())
            .await
            .unwrap();
        assert_eq!(stored.note.note_type, NoteType::Note);
        assert_eq!(stored.note.source_channel, "matrix");

        let err = run(
            &services,
            &owner,
            "mcp",
            "knowledge_create_note",
            json!({"title": "Bad", "content": "body", "note_type": "poem"}),
        )
        .await
        .unwrap_err();
        assert!(err.contains("poem"), "{err}");
    }

    #[tokio::test]
    async fn get_note_returns_the_markdown_with_title_type_and_tags() {
        let (services, owner) = setup("kt_get").await;
        let created = run(
            &services,
            &owner,
            "mcp",
            "knowledge_create_note",
            json!({"title": "Recipe", "content": "- eggs
- 面粉", "tags": ["Food"], "note_type": "fact"}),
        )
        .await
        .unwrap();

        let got = run(
            &services,
            &owner,
            "mcp",
            "knowledge_get_note",
            json!({"id": created["id"]}),
        )
        .await
        .unwrap();

        assert_eq!(got["title"], "Recipe");
        assert_eq!(got["note_type"], "fact");
        assert_eq!(got["tags"], json!(["food"]));
        assert_eq!(
            got["content"],
            "- eggs
- 面粉"
        );
        assert_eq!(got["truncated"], false);
    }

    #[tokio::test]
    async fn get_note_cuts_a_long_body_and_says_so() {
        let (services, owner) = setup("kt_get_long").await;
        let body = "字".repeat(GET_NOTE_MAX_CHARS + 500);
        let created = services
            .notes
            .create(&owner, NewNote::new("Long", body.clone()))
            .await
            .unwrap();

        let got = run(
            &services,
            &owner,
            "mcp",
            "knowledge_get_note",
            json!({"id": created.id.0}),
        )
        .await
        .unwrap();

        assert_eq!(got["truncated"], true);
        assert_eq!(
            got["content"].as_str().unwrap().chars().count(),
            GET_NOTE_MAX_CHARS
        );
        assert_eq!(got["total_chars"], body.chars().count());
    }

    #[tokio::test]
    async fn get_note_cannot_read_another_accounts_note() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let (alice, bob) = (
            AccountId::from_seed("kt_get_alice"),
            AccountId::from_seed("kt_get_bob"),
        );
        crate::db::pool::seed_account(&pool, &alice).await;
        crate::db::pool::seed_account(&pool, &bob).await;
        let services = AppServices::new(&pool);
        let secret = services
            .notes
            .create(&bob, NewNote::new("Bob private", "bob only"))
            .await
            .unwrap();

        // The model may name any id; the owner still comes from the turn.
        for args in [
            json!({"id": secret.id.0}),
            json!({"id": secret.id.0, "owner_id": bob.to_string()}),
        ] {
            let err = run(&services, &alice, "mcp", "knowledge_get_note", args)
                .await
                .unwrap_err();
            assert!(
                !err.contains("bob only") && !err.contains("Bob private"),
                "{err}"
            );
        }
    }

    #[tokio::test]
    async fn get_note_requires_an_integer_id() {
        let (services, owner) = setup("kt_get_args").await;
        let err = run(&services, &owner, "mcp", "knowledge_get_note", json!({}))
            .await
            .unwrap_err();
        assert!(err.contains("'id'"), "{err}");
    }
}
