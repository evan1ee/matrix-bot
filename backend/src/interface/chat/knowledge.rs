//! Chat interface for Second Brain knowledge base & notes.
//!
//! Handles `!note` / `/note`, `!notes` / `/notes`, and `!search` / `/search`
//! commands via `NoteService` and `SearchService`.

use async_trait::async_trait;
use tracing::error;

use crate::application::AppServices;
use crate::db::Database;
use crate::domain::accounts::AccountId;
use crate::domain::knowledge::note::NoteFormat;
use crate::domain::knowledge::search::KnowledgeItemType;
use crate::interface::channels::message::Reply;
use crate::interface::chat::{error_reply, BotModule, CommandContext, CommandHelp};
use crate::utils::html::html_escape;

pub const CHAT_INPUT_MAX: usize = 500;

pub struct KnowledgeModule;

impl KnowledgeModule {
    pub fn new() -> Self {
        Self
    }

    async fn respond(&self, ctx: &CommandContext, services: &AppServices) -> Option<String> {
        let cmd = ctx.command.to_ascii_lowercase();
        if cmd == "notes" {
            let tag = ctx.raw_args.trim();
            let tag_opt = if tag.is_empty() { None } else { Some(tag) };
            Some(handle_list(services, &ctx.owner_id, tag_opt).await)
        } else if cmd == "note" {
            let args = ctx.raw_args.trim();
            if args.is_empty() {
                if let Some(quoted) = &ctx.quoted {
                    Some(handle_quoted_note(services, &ctx.owner_id, quoted, ctx).await)
                } else {
                    Some(format_help())
                }
            } else if let Some(rest) = args.strip_prefix("add") {
                Some(handle_add(services, &ctx.owner_id, rest.trim(), ctx).await)
            } else if let Some(rest) = args.strip_prefix("list") {
                let tag = rest.trim();
                let tag_opt = if tag.is_empty() { None } else { Some(tag) };
                Some(handle_list(services, &ctx.owner_id, tag_opt).await)
            } else if let Some(rest) = args.strip_prefix("search") {
                Some(handle_search(services, &ctx.owner_id, rest.trim()).await)
            } else {
                Some(handle_quick_note(services, &ctx.owner_id, args, ctx).await)
            }
        } else if cmd == "search" {
            let args = ctx.raw_args.trim();
            if args.is_empty() {
                Some("🔍 <b>Usage:</b> <code>!search &lt;query&gt;</code>".to_string())
            } else {
                Some(handle_search(services, &ctx.owner_id, args).await)
            }
        } else {
            None
        }
    }
}

impl Default for KnowledgeModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BotModule for KnowledgeModule {
    fn name(&self) -> &str {
        "Knowledge"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "note".to_string(),
                usage: "!note add <title> | <content> or !note <text>".to_string(),
                description: "Create or manage notes in your Second Brain".to_string(),
            },
            CommandHelp {
                name: "notes".to_string(),
                usage: "!notes [tag]".to_string(),
                description: "List recent notes".to_string(),
            },
            CommandHelp {
                name: "search".to_string(),
                usage: "!search <query>".to_string(),
                description: "Hybrid search across notes, bookmarks, journal, and footprints".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &AppServices,
        _db: &Database,
    ) -> Option<Reply> {
        let html = self.respond(ctx, services).await?;
        Some(Reply::html(html))
    }
}

fn format_help() -> String {
    "📝 <b>Second Brain & Notes:</b><br/>\
     • <code>!note add &lt;title&gt; | &lt;content&gt;</code> — Create note with title<br/>\
     • <code>!note &lt;text&gt;</code> — Quick note capture<br/>\
     • <code>!notes [tag]</code> — List recent notes<br/>\
     • <code>!search &lt;query&gt;</code> — Hybrid search across knowledge base"
        .to_string()
}

fn extract_tags(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|w| {
            if let Some(tag) = w.strip_prefix('#') {
                let cleaned: String = tag
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                    .collect();
                if !cleaned.is_empty() {
                    Some(cleaned.to_lowercase())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
}

fn split_title_content(text: &str) -> (String, String) {
    if let Some((title, content)) = text.split_once('|') {
        (title.trim().to_string(), content.trim().to_string())
    } else {
        let lines: Vec<&str> = text.lines().collect();
        let first_line = lines.first().copied().unwrap_or(text).trim();
        let title = if first_line.chars().count() > 50 {
            format!("{}...", first_line.chars().take(47).collect::<String>())
        } else {
            first_line.to_string()
        };
        (title, text.trim().to_string())
    }
}

async fn handle_add(
    services: &AppServices,
    owner: &AccountId,
    args: &str,
    ctx: &CommandContext,
) -> String {
    if args.is_empty() {
        return "❌ <b>Usage:</b> <code>!note add &lt;title&gt; | &lt;content&gt;</code>".to_string();
    }
    let (title, content) = split_title_content(args);
    let tags = extract_tags(args);
    let channel = ctx.channel().to_string();
    let room_key = ctx.room_key();
    match services
        .notes
        .create_note(
            owner,
            &title,
            &content,
            Some(NoteFormat::Markdown),
            Some(&channel),
            Some(&room_key),
            Some(&tags),
        )
        .await
    {
        Ok(note) => format!(
            "<p>📝 <b>Note #{} saved:</b> {}</p>",
            note.id.0,
            html_escape(&note.title)
        ),
        Err(err) => error_reply(err).html_body().to_string(),
    }
}

async fn handle_quick_note(
    services: &AppServices,
    owner: &AccountId,
    args: &str,
    ctx: &CommandContext,
) -> String {
    let (title, content) = split_title_content(args);
    let tags = extract_tags(args);
    let channel = ctx.channel().to_string();
    let room_key = ctx.room_key();
    match services
        .notes
        .create_note(
            owner,
            &title,
            &content,
            Some(NoteFormat::Markdown),
            Some(&channel),
            Some(&room_key),
            Some(&tags),
        )
        .await
    {
        Ok(note) => format!(
            "<p>📝 <b>Note #{} captured:</b> {}</p>",
            note.id.0,
            html_escape(&note.title)
        ),
        Err(err) => error_reply(err).html_body().to_string(),
    }
}

async fn handle_quoted_note(
    services: &AppServices,
    owner: &AccountId,
    quoted: &crate::domain::chat::QuotedMessage,
    ctx: &CommandContext,
) -> String {
    let author = quoted.author.as_deref().unwrap_or("Quote");
    let title = format!("Quote from {}", author);
    let content = format!("> {}\n", quoted.body.replace('\n', "\n> "));
    let tags = vec!["quote".to_string()];
    let channel = ctx.channel().to_string();
    let room_key = ctx.room_key();
    match services
        .notes
        .create_note(
            owner,
            &title,
            &content,
            Some(NoteFormat::Markdown),
            Some(&channel),
            Some(&room_key),
            Some(&tags),
        )
        .await
    {
        Ok(note) => format!(
            "<p>📝 <b>Note #{} saved from quote:</b></p><blockquote>{}</blockquote>",
            note.id.0,
            html_escape(&quoted.body)
        ),
        Err(err) => error_reply(err).html_body().to_string(),
    }
}

async fn handle_list(services: &AppServices, owner: &AccountId, tag: Option<&str>) -> String {
    match services.notes.list_notes(owner, tag, false, 10).await {
        Ok(notes) if notes.is_empty() => {
            if let Some(t) = tag {
                format!(
                    "<p>📝 <i>No notes found with tag <code>#{}</code>.</i></p>",
                    html_escape(t)
                )
            } else {
                "<p>📝 <i>No notes found. Create one with <code>!note add &lt;title&gt; | &lt;content&gt;</code></i></p>".to_string()
            }
        }
        Ok(notes) => {
            let mut html = String::from("<p>📝 <b>Your Recent Notes:</b></p><ul>");
            for n in notes {
                let snippet = if n.content.chars().count() > 60 {
                    format!("{}...", n.content.chars().take(57).collect::<String>())
                } else {
                    n.content.clone()
                };
                let tags_str = if n.tags.is_empty() {
                    String::new()
                } else {
                    format!(" <code>[{}]</code>", n.tags.join(", "))
                };
                html.push_str(&format!(
                    "<li><b>#{} {}</b>{}<br/><i>{}</i></li>",
                    n.id.0,
                    html_escape(&n.title),
                    html_escape(&tags_str),
                    html_escape(&snippet)
                ));
            }
            html.push_str("</ul>");
            html
        }
        Err(err) => error_reply(err).html_body().to_string(),
    }
}

async fn handle_search(services: &AppServices, owner: &AccountId, query: &str) -> String {
    if query.is_empty() {
        return "🔍 <b>Usage:</b> <code>!search &lt;query&gt;</code>".to_string();
    }
    match services.knowledge_search.search(owner, query, None, 3).await {
        Ok(results) if results.is_empty() => {
            format!(
                "<p>🔍 <i>No knowledge items found matching &quot;{}&quot;.</i></p>",
                html_escape(query)
            )
        }
        Ok(results) => {
            let mut html = format!(
                "<p>🔍 <b>Search Results</b> for <i>&quot;{}&quot;</i>:</p><ol>",
                html_escape(query)
            );
            for item in results {
                let badge = match item.item_type {
                    KnowledgeItemType::Note => "📝 Note",
                    KnowledgeItemType::Bookmark => "🔖 Bookmark",
                    KnowledgeItemType::Journal => "📔 Journal",
                    KnowledgeItemType::Footprint => "📍 Place",
                };
                let score_pct = (item.similarity_score.clamp(0.0, 1.0) * 100.0) as u32;
                html.push_str(&format!(
                    "<li><b>[{}] {}</b> (relevance: {}%)<br/><i>{}</i></li>",
                    badge,
                    html_escape(&item.title),
                    score_pct,
                    html_escape(&item.content_snippet)
                ));
            }
            html.push_str("</ol>");
            html
        }
        Err(err) => error_reply(err).html_body().to_string(),
    }
}

pub async fn capture_photo_note(
    services: &AppServices,
    room_id: &str,
    owner_id: &AccountId,
    caption: &str,
    _mimetype: Option<&str>,
    media_bytes: &[u8],
    ocr: &crate::infrastructure::inference::ocr::OcrClient,
) -> String {
    let text = match ocr.scan(media_bytes.to_vec()).await {
        Ok(s) if !s.text.trim().is_empty() => s.text.trim().to_string(),
        Ok(_) => return "⚠️ <b>图片识别失败</b><br/>• <b>原因:</b> 未能从图片中识别出有效文字。".to_string(),
        Err(e) => {
            error!("OCR extraction failed for photo note: {e:?}");
            return "⚠️ <b>图片识别不可用</b><br/>• <b>原因:</b> OCR 识别服务暂时不可用。".to_string();
        }
    };

    let user_title = caption
        .strip_prefix("/note")
        .or_else(|| caption.strip_prefix("!note"))
        .unwrap_or("")
        .trim();
    let title = if user_title.is_empty() {
        let first_line = text.lines().next().unwrap_or("Photo Note");
        if first_line.chars().count() > 50 {
            format!("{}...", first_line.chars().take(47).collect::<String>())
        } else {
            first_line.to_string()
        }
    } else {
        user_title.to_string()
    };

    let content = format!("> {}\n", text.replace('\n', "\n> "));
    let tags = vec!["photo".to_string(), "ocr".to_string()];
    match services
        .notes
        .create_note(
            owner_id,
            &title,
            &content,
            Some(NoteFormat::Markdown),
            Some("matrix"),
            Some(room_id),
            Some(&tags),
        )
        .await
    {
        Ok(note) => format!(
            "<p>📝 <b>Photo Note #{} Captured</b></p><p><b>Title:</b> {}</p><blockquote>{}</blockquote>",
            note.id.0,
            html_escape(&note.title),
            html_escape(&text),
        ),
        Err(err) => error_reply(err).html_body().to_string(),
    }
}

pub async fn capture_voice_note(
    services: &AppServices,
    room_id: &str,
    owner_id: &AccountId,
    caption: &str,
    media_bytes: &[u8],
    mimetype: Option<&str>,
    voice_config: &crate::interface::chat::voice::VoiceConfig,
    stt: &crate::infrastructure::inference::stt::SttClient,
    progress: Option<&dyn crate::llm::progress::ProgressSink>,
) -> String {
    if let Some(progress) = progress {
        progress.report_stage("🎧 正在转写语音笔记...").await;
    }
    let transcription = match stt
        .transcribe(media_bytes.to_vec(), mimetype, voice_config.language_hint())
        .await
    {
        Ok(t) if !t.text.trim().is_empty() => t.text.trim().to_string(),
        Ok(_) => return "⚠️ <b>语音未能识别</b><br/>• <b>原因:</b> 未检测到清晰人声。".to_string(),
        Err(e) => {
            error!("STT transcription failed for voice note: {e:?}");
            return "⚠️ <b>语音识别不可用</b><br/>• <b>原因:</b> STT 服务暂时不可用。".to_string();
        }
    };

    let user_title = caption
        .strip_prefix("/note")
        .or_else(|| caption.strip_prefix("!note"))
        .unwrap_or("")
        .trim();
    let title = if user_title.is_empty() {
        let first_sentence = transcription
            .split(['。', '.', '!', '?', '\n'])
            .next()
            .unwrap_or("Voice Note");
        if first_sentence.chars().count() > 50 {
            format!("{}...", first_sentence.chars().take(47).collect::<String>())
        } else {
            first_sentence.to_string()
        }
    } else {
        user_title.to_string()
    };

    let tags = vec!["voice".to_string()];
    match services
        .notes
        .create_note(
            owner_id,
            &title,
            &transcription,
            Some(NoteFormat::Markdown),
            Some("matrix"),
            Some(room_id),
            Some(&tags),
        )
        .await
    {
        Ok(note) => format!(
            "<p>🎙️ <b>Voice Note #{} Captured</b></p><p><b>Title:</b> {}</p><p><i>&quot;{}&quot;</i></p>",
            note.id.0,
            html_escape(&note.title),
            html_escape(&transcription),
        ),
        Err(err) => error_reply(err).html_body().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};
    use crate::domain::channels::{ConversationAddress, PlatformIdentity};
    use crate::domain::chat::QuotedMessage;
    use std::sync::Arc;

    fn mock_context(command: &str, raw_args: &str, owner_id: AccountId) -> CommandContext {
        CommandContext {
            conversation: ConversationAddress::Matrix("!test:matrix.org".to_string()),
            owner_id,
            sender: PlatformIdentity::matrix("@alice:matrix.org").unwrap(),
            command: command.to_string(),
            raw_args: raw_args.to_string(),
            full_body: format!("!{} {}", command, raw_args),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            progress: None,
            outbound: None,
        }
    }

    #[tokio::test]
    async fn test_knowledge_module_commands_and_help() {
        let module = KnowledgeModule::new();
        assert_eq!(module.name(), "Knowledge");
        let cmds = module.commands();
        assert!(cmds.iter().any(|c| c.name == "note"));
        assert!(cmds.iter().any(|c| c.name == "notes"));
        assert!(cmds.iter().any(|c| c.name == "search"));
    }

    #[tokio::test]
    async fn test_knowledge_module_note_add_and_list() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let mock_embedder = Arc::new(crate::infrastructure::inference::embedding::MockEmbeddingProvider::new(384));
        let services = AppServices::new(&pool).with_embedder(mock_embedder);
        let db = Database { pool };
        let module = KnowledgeModule::new();

        // 1. Add note
        let ctx = mock_context("note", "add Clean Architecture | Separation of concerns #architecture", alice);
        let reply = module.handle_command(&ctx, &services, &db).await.expect("reply");
        assert!(reply.html_body().contains("Note #"));
        assert!(reply.html_body().contains("Clean Architecture"));

        // 2. Quick note
        let ctx_quick = mock_context("note", "Just a quick thought about Rust #rust", alice);
        let reply_quick = module.handle_command(&ctx_quick, &services, &db).await.expect("reply");
        assert!(reply_quick.html_body().contains("captured"));

        // 3. List notes
        let ctx_list = mock_context("notes", "", alice);
        let reply_list = module.handle_command(&ctx_list, &services, &db).await.expect("reply");
        assert!(reply_list.html_body().contains("Clean Architecture"));
        assert!(reply_list.html_body().contains("Just a quick thought"));

        // 4. List notes by tag
        let ctx_tag = mock_context("notes", "architecture", alice);
        let reply_tag = module.handle_command(&ctx_tag, &services, &db).await.expect("reply");
        assert!(reply_tag.html_body().contains("Clean Architecture"));

        // 5. Search before embedding returns no match message
        let ctx_search = mock_context("search", "Separation", alice);
        let reply_search = module.handle_command(&ctx_search, &services, &db).await.expect("reply");
        assert!(reply_search.html_body().contains("No knowledge items found"));

        // After embedding the note, search returns results
        let notes = services.notes.list_notes(&alice, None, false, 10).await.unwrap();
        services
            .knowledge_search
            .embed_item(&alice, crate::domain::knowledge::search::KnowledgeItemType::Note, notes[0].id.0)
            .await
            .expect("embed note");
        let reply_search_2 = module.handle_command(&ctx_search, &services, &db).await.expect("reply");
        assert!(reply_search_2.html_body().contains("Search Results"));
    }

    #[tokio::test]
    async fn test_knowledge_module_quoted_note() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let mock_embedder = Arc::new(crate::infrastructure::inference::embedding::MockEmbeddingProvider::new(384));
        let services = AppServices::new(&pool).with_embedder(mock_embedder);
        let db = Database { pool };
        let module = KnowledgeModule::new();

        let mut ctx = mock_context("note", "", alice);
        ctx.quoted = Some(QuotedMessage {
            author: Some("@bob:matrix.org".to_string()),
            body: "Make invalid states unrepresentable.".to_string(),
            event_id: None,
        });

        let reply = module.handle_command(&ctx, &services, &db).await.expect("reply");
        assert!(reply.html_body().contains("saved from quote"));
        assert!(reply.html_body().contains("Make invalid states unrepresentable."));
    }
}
