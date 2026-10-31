# Briefing podcast (Blueprint 6) — plan

Spec: `specs/2026-10-31-feature-blueprints.md` § Blueprint 6. Rulings that deviate from the spec:

- **No `briefing_sent_days` table exists** (the sent day lives in `user_settings.last_briefing_date`), so migration
  `0018_briefing_audio.sql` creates `briefing_audio (owner_id, local_date, storage_key)` instead of an `ALTER`.
  (0017 is taken by `knowledge_note_links`.) It is owner-keyed: listed in `OWNED_TABLES`, merge keeps the survivor's row.
- **The briefing is template-built, not LLM-written**, so the `<audio_script>` is built by a pure function from the same
  data as the text (`application/briefing/audio_script.rs`) and capped by the domain type `BriefingAudioScript` (2,000 chars).
- **No storage sweep exists.** Retention is enforced by the owner's own daily `briefing.send` job: it deletes that
  owner's rows (and objects) older than 30 days. No new worker.
- Only the scheduled job produces audio; the on-demand trigger stays text-only.

## Slices (test first each)
1. Domain `BriefingAudioScript` (cap, trim, sentence-boundary truncation).
2. Migration 0018, repository + `BriefingService::{record_audio, audio_key, prune_audio}`, `OWNED_TABLES`, merge collision.
3. Script builder wired into `BriefingPayload.audio_script` (not exposed on the wire).
4. Job: synthesize (fail-open) → store → attach → record → prune; tests with mock TTS, failing TTS, no TTS.
5. `GET /api/v1/briefing/audio?date=` (presigned, owner-scoped), OpenAPI + generated types.
6. Frontend `BriefingCard` + `useBriefingAudio`, fake-api mock, Playwright spec.
7. Gates, HANDOFF, one commit.
