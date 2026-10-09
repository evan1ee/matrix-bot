//! Export and import of notes as Markdown files (see `note_file`).
//!
//! Both are synchronous and sized for a personal vault. Import runs every
//! note through the normal write path, so each one publishes its usual event
//! and embedding job; an id in a file is never trusted across accounts.

use tracing::error;

use super::note_file::{export_slug, parse_note_file, render_note_file, ParsedNote};
use super::NoteService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::knowledge::note::{normalize_tags, NewNote, NoteDocument, NotePatch, NoteType};
use crate::infrastructure::postgres::repositories;

/// Most notes one export holds. A vault past this needs a background export.
pub const MAX_EXPORT_NOTES: usize = 10_000;

/// One file of an export: a path inside the archive and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFile {
    pub path: String,
    pub text: String,
}

/// One file offered to an import: its name in the archive and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFile {
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFile {
    pub file: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub created: usize,
    pub updated: usize,
    pub skipped: Vec<SkippedFile>,
}

enum Outcome {
    Created,
    Updated,
    Unchanged,
}

impl NoteService {
    /// Every note of `owner`, archived included, as `notes/<slug>-<id>.md`.
    pub async fn export(&self, owner: &AccountId) -> Result<Vec<ExportFile>, AppError> {
        self.export_up_to(owner, MAX_EXPORT_NOTES).await
    }

    async fn export_up_to(
        &self,
        owner: &AccountId,
        max_notes: usize,
    ) -> Result<Vec<ExportFile>, AppError> {
        let fetch = i64::try_from(max_notes)
            .unwrap_or(i64::MAX)
            .saturating_add(1);
        let documents = repositories::knowledge::export_documents(&self.pool, owner, fetch).await?;
        if documents.len() > max_notes {
            return Err(AppError::Validation(format!(
                "too many notes to export at once (limit {max_notes})"
            )));
        }
        Ok(documents.iter().map(export_file).collect())
    }

    /// Creates or updates one note per file. A file that cannot be imported is
    /// reported in `skipped` and never stops the others. Each note is written
    /// in its own transaction, so one bad file cannot roll back its neighbours.
    pub async fn import(&self, owner: &AccountId, files: Vec<ImportFile>) -> ImportReport {
        let mut report = ImportReport::default();
        for file in files {
            let parsed = parse_note_file(&file.name, &file.text);
            match self.import_one(owner, parsed).await {
                Ok(Outcome::Created) => report.created += 1,
                Ok(Outcome::Updated) => report.updated += 1,
                Ok(Outcome::Unchanged) => report.skipped.push(skipped(file.name, "unchanged")),
                Err(err) => {
                    let reason = reason_for(&file.name, err);
                    report.skipped.push(skipped(file.name, &reason));
                }
            }
        }
        report
    }

    async fn import_one(&self, owner: &AccountId, parsed: ParsedNote) -> Result<Outcome, AppError> {
        let note_type = parsed
            .note_type
            .as_deref()
            .map(str::parse::<NoteType>)
            .transpose()?;
        // An id counts only when this account owns it; anything else is a new note.
        if let Some(id) = parsed.id {
            match self.get(owner, id).await {
                Ok(current) => {
                    return self
                        .import_existing(owner, current, parsed, note_type)
                        .await
                }
                Err(AppError::NotFound(_)) => {}
                Err(other) => return Err(other),
            }
        }
        // The channel is "web": an import is not a capture from the file's
        // original channel, but its reference is kept.
        let new = NewNote::new(parsed.title, parsed.markdown)
            .note_type(note_type.unwrap_or_default())
            .source("web", parsed.source_ref)
            .tags(parsed.tags)
            .archived(parsed.archived.unwrap_or(false));
        self.create(owner, new).await?;
        Ok(Outcome::Created)
    }

    async fn import_existing(
        &self,
        owner: &AccountId,
        current: NoteDocument,
        parsed: ParsedNote,
        note_type: Option<NoteType>,
    ) -> Result<Outcome, AppError> {
        let tags = normalize_tags(&parsed.tags);
        let same = current.markdown == parsed.markdown
            && current.note.title == parsed.title.trim()
            && current.note.tags == tags
            && note_type.is_none_or(|t| t == current.note.note_type)
            && parsed
                .archived
                .is_none_or(|a| a == current.note.is_archived);
        if same {
            return Ok(Outcome::Unchanged);
        }
        let patch = NotePatch {
            title: Some(parsed.title),
            markdown: Some(parsed.markdown),
            tags: Some(parsed.tags),
            note_type,
            is_archived: parsed.archived,
            ..NotePatch::default()
        };
        self.update(owner, current.note.id.0, patch).await?;
        Ok(Outcome::Updated)
    }
}

fn export_file(document: &NoteDocument) -> ExportFile {
    let note = &document.note;
    ExportFile {
        path: format!("notes/{}.md", export_slug(&note.title, note.id.0)),
        text: render_note_file(document),
    }
}

fn skipped(file: String, reason: &str) -> SkippedFile {
    SkippedFile {
        file,
        reason: reason.to_string(),
    }
}

/// What the person importing may read about a failure: a rule they broke, never
/// a database error.
fn reason_for(file: &str, err: AppError) -> String {
    match err {
        AppError::Validation(message)
        | AppError::NotFound(message)
        | AppError::Conflict(message)
        | AppError::Forbidden(message) => message,
        other => {
            error!(operation = "knowledge.import", file, error = %other, "note import failed");
            "could not be saved".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::AppServices;

    async fn services_with(seeds: &[&str]) -> (AppServices, Vec<AccountId>) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owners: Vec<AccountId> = seeds.iter().map(|s| AccountId::from_seed(s)).collect();
        for owner in &owners {
            crate::db::pool::seed_account(&pool, owner).await;
        }
        (AppServices::new(&pool), owners)
    }

    fn file(name: &str, text: &str) -> ImportFile {
        ImportFile {
            name: name.to_string(),
            text: text.to_string(),
        }
    }

    async fn all_notes(
        services: &AppServices,
        owner: &AccountId,
    ) -> Vec<crate::domain::knowledge::note::Note> {
        services
            .notes
            .list(owner, None, None, true, 100)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn exporting_and_importing_into_a_fresh_account_keeps_every_note() {
        let (services, owners) = services_with(&["xfer_src", "xfer_dst"]).await;
        let (src, dst) = (&owners[0], &owners[1]);
        let bodies = [
            "# One\n\nbody",
            "\n\nleading blanks\n",
            "猫坐在垫子上\r\n- 面粉",
        ];
        for (index, body) in bodies.iter().enumerate() {
            services
                .notes
                .create(
                    src,
                    NewNote::new(format!("Note {index}: \"q\" 猫"), *body)
                        .note_type(NoteType::Fact)
                        .tags(vec!["alpha".into(), "beta".into()]),
                )
                .await
                .unwrap();
        }
        let archived = services
            .notes
            .create(src, NewNote::new("Archived", "kept in the export"))
            .await
            .unwrap();
        services
            .notes
            .archive_note(src, archived.id.0, true)
            .await
            .unwrap();

        let exported = services.notes.export(src).await.unwrap();
        assert_eq!(exported.len(), 4);
        assert!(exported
            .iter()
            .all(|f| f.path.starts_with("notes/") && f.path.ends_with(".md")));

        let files = exported.iter().map(|f| file(&f.path, &f.text)).collect();
        let report = services.notes.import(dst, files).await;
        assert_eq!(
            (report.created, report.updated),
            (4, 0),
            "{:?}",
            report.skipped
        );
        assert!(report.skipped.is_empty());

        let mut original = all_notes(&services, src).await;
        let mut copy = all_notes(&services, dst).await;
        original.sort_by_key(|n| n.title.clone());
        copy.sort_by_key(|n| n.title.clone());
        assert_eq!(copy.len(), 4);
        for (a, b) in original.iter().zip(&copy) {
            assert_eq!(
                (&a.title, a.note_type, &a.tags),
                (&b.title, b.note_type, &b.tags)
            );
            assert_eq!(a.content_hash, b.content_hash, "{} body differs", a.title);
            assert_ne!(a.id, b.id);
            assert_eq!(a.is_archived, b.is_archived, "{} archived state", a.title);
        }
        assert!(copy.iter().any(|n| n.title == "Archived" && n.is_archived));
    }

    #[tokio::test]
    async fn an_owned_id_updates_in_place_and_an_unchanged_file_is_skipped() {
        let (services, owners) = services_with(&["xfer_upd"]).await;
        let owner = &owners[0];
        let note = services
            .notes
            .create(owner, NewNote::new("Draft", "v1").tags(vec!["t".into()]))
            .await
            .unwrap();
        let exported = services.notes.export(owner).await.unwrap();

        let same = services
            .notes
            .import(owner, vec![file(&exported[0].path, &exported[0].text)])
            .await;
        assert_eq!((same.created, same.updated), (0, 0));
        assert_eq!(same.skipped[0].reason, "unchanged");

        let edited = exported[0].text.replace("v1", "v2");
        let report = services
            .notes
            .import(owner, vec![file("a.md", &edited)])
            .await;
        assert_eq!((report.created, report.updated), (0, 1));
        let stored = services.notes.get(owner, note.id.0).await.unwrap();
        assert_eq!(stored.markdown, "v2");
        assert_eq!(all_notes(&services, owner).await.len(), 1);
    }

    #[tokio::test]
    async fn a_foreign_or_missing_id_creates_a_note_and_never_touches_the_other_account() {
        let (services, owners) = services_with(&["xfer_alice", "xfer_bob"]).await;
        let (alice, bob) = (&owners[0], &owners[1]);
        let bobs = services
            .notes
            .create(bob, NewNote::new("Bob private", "bob body"))
            .await
            .unwrap();
        let hostile = format!(
            "---\nid: {}\ntitle: \"Overwrite\"\n---\n\nhijacked",
            bobs.id.0
        );
        let missing = "---\nid: 999999999\ntitle: \"Ghost\"\n---\n\nghost body";

        let report = services
            .notes
            .import(alice, vec![file("x.md", &hostile), file("y.md", missing)])
            .await;

        assert_eq!((report.created, report.updated), (2, 0));
        let untouched = services.notes.get(bob, bobs.id.0).await.unwrap();
        assert_eq!(untouched.note.title, "Bob private");
        assert_eq!(untouched.markdown, "bob body");
        assert_eq!(all_notes(&services, alice).await.len(), 2);
    }

    #[tokio::test]
    async fn bad_files_are_reported_and_the_rest_still_import() {
        let (services, owners) = services_with(&["xfer_bad"]).await;
        let owner = &owners[0];
        let long_title = format!("---\ntitle: \"{}\"\n---\nbody", "t".repeat(300));
        let files = vec![
            file("empty.md", "---\ntitle: Empty\n---\n"),
            file("type.md", "---\ntype: poem\ntitle: T\n---\nbody"),
            file("long.md", &long_title),
            file("good.md", "# Good\nbody"),
        ];

        let report = services.notes.import(owner, files).await;

        assert_eq!((report.created, report.updated), (1, 0));
        let names: Vec<&str> = report.skipped.iter().map(|s| s.file.as_str()).collect();
        assert_eq!(names, ["empty.md", "type.md", "long.md"]);
        assert!(
            report.skipped[1].reason.contains("poem"),
            "{:?}",
            report.skipped[1]
        );
        assert!(report
            .skipped
            .iter()
            .all(|s| !s.reason.contains("database")));
    }

    #[tokio::test]
    async fn an_export_over_the_cap_is_refused_and_names_the_cap() {
        let (services, owners) = services_with(&["xfer_cap"]).await;
        let owner = &owners[0];
        for n in 0..3 {
            services
                .notes
                .create(owner, NewNote::new(format!("n{n}"), "body"))
                .await
                .unwrap();
        }
        let err = services.notes.export_up_to(owner, 2).await.unwrap_err();
        assert!(
            matches!(err, AppError::Validation(ref m) if m.contains('2')),
            "{err}"
        );
        assert_eq!(
            services.notes.export_up_to(owner, 3).await.unwrap().len(),
            3
        );
    }

    #[tokio::test]
    async fn an_export_holds_only_the_callers_notes() {
        let (services, owners) = services_with(&["xfer_iso_a", "xfer_iso_b"]).await;
        services
            .notes
            .create(&owners[1], NewNote::new("Not yours", "secret"))
            .await
            .unwrap();
        assert!(services.notes.export(&owners[0]).await.unwrap().is_empty());
    }
}
