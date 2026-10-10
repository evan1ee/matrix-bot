# Future Feature Enhancements, Refinements & Brand New Horizons

This document captures architectural ideation, potential feature enhancements, and brand new conceptual horizons for **Matrix Bot**, building upon the completed PostgreSQL 18 layered foundation, omnichannel adapters (Matrix, Discord, Telegram, Email), decoupled object storage, durable job runtime, autonomous agent engine (native tool calling with step-up approvals), and local ONNX inference engine (multilingual-e5-small, Kokoro-82M TTS, Sherpa/SenseVoice STT).

---

## 1. Brand New Conceptual Horizons (Emergent AI Superpowers)

These ideas move beyond standard CRUD productivity tracking into **active life intelligence**, leveraging the unique convergence of 24/7 background execution, omnichannel presence, local ONNX audio/vision, and full personal life data in PostgreSQL 18.

### 1.1 The Synthetic "Advisory Board" & Devil's Advocate
*Deliberation grounded in your actual financial reality, historical failures, and life goals.*

- **The Problem:** General LLM chats provide generic optimism or toothless advice when evaluating big decisions because they lack context on your real assets, past patterns, and commitments.
- **The Concept:** You invoke `!convene "Should I spend $3,000 to attend Conference X?"` or `!debate "Allocating 25% of portfolio to high-risk assets"`. The bot orchestrates a simulated multi-agent council in your chat or web console:
  1. **The Chief Risk Officer:** Queries your real monthly burn rate from `recurring_bills` and available cash reserve from `finance` to evaluate true financial affordability.
  2. **The Historical Skeptic:** Queries `pgvector` across `life_journal` and `notes` for past times you made similar commitments (e.g. *"In 2024 you bought course Y and recorded in your journal that you only finished 10%"*).
  3. **The Strategist / Advocate:** Evaluates alignment against long-term goals and current active projects in `notes`.
- **Output:** A structured **"Board Resolution"** card in chat (with vote tally, risk vectors, and direct links to your own historical notes) or a detailed deliberation artifact in the web UI.

### 1.2 Ambient Life Telemetry & Temporal Correlation Engine
*Discovering hidden causal links between your physical location, money, mood, and habits.*

- **The Problem:** Footprints, habits, expenses, and mood are tracked in silos, but real-world behaviors strongly correlate across domains.
- **The Concept:** A weekly background analytics job correlates time-series data across disparate domain tables to surface actionable personal insights:
  - *"Insight: On days you visit Venue X (e.g. downtown cafe), your discretionary spending is 40% higher and evening habit completion drops by half."*
  - *"Productivity Trigger: Your high-priority todo completion rate is 3× higher on days you log a mood entry before 09:00."*
  - *"Downtime Alert: You haven't recorded a footprint outside your home in 4 days, and your logged journal sentiment dropped from 4 to 2. Nudge: Time for a walk?"*
- **Architectural Fit:** 100% private, executed as a scheduled read-only background job (`interface/jobs/telemetry.rs`) querying PostgreSQL 18 directly, without any personal data leaving the machine.

### 1.3 The "Serendipity Engine" & Dead Project Resurrector
*Turning a digital graveyard of notes and bookmarks into creative collision and actionable triage.*

- **The Problem:** Second brains inevitably accumulate hundreds of unread bookmarks, stalled project notes, and abandoned todos.
- **The Concept:**
  - **Cross-Domain Collision:** Every weekend, the background agent clusters embeddings (`vector(384)`) across `notes` and `bookmarks` created months apart with zero shared tags. When it detects an unexpected conceptual parallel, it delivers a **"Serendipity Spark"**:
    > 💡 *Serendipity Spark: In March you saved an article on "Event-Driven Microservices", and yesterday you wrote a note about "Modular Synthesizers". Both explore unidirectional reactive pipelines. Here's a brief synthesis on how they connect...*
  - **Stale Commitment Audit:** Flags high-priority todos or project notes untouched for 60+ days and presents 3 clean choices:
    1. *Archive with dignity* (clean up the clutter).
    2. *Break down into a 5-minute atomic starter action*.
    3. *Have the AI summarize the background research into an actionable brief*.

### 1.4 Dynamic "Life War Room" / Crisis Incident Command
*Ephemeral, high-intensity workspaces for real-world chaotic events.*

- **The Problem:** When real life goes wrong (flight cancelled, unexpected hospital visit, moving apartments, sudden emergency), managing tasks and information across disparate apps creates severe cognitive overload.
- **The Concept:** You send `!warroom "Flight Stranded in Tokyo"`.
- **What Happens:**
  1. Instantly provisions a dedicated ephemeral Matrix room or Discord channel titled `[WAR ROOM] Tokyo Flight Incident`.
  2. Pulls relevant context (flight confirmation emails from `channels/email`, passport note from `knowledge`, airline bookmarks, travel insurance policy) into the room topic and pinned messages.
  3. Deploys a dedicated incident subagent that:
     - Maintains a real-time event timeline and hard compensation/rebooking deadlines.
     - Categorizes incoming receipt photos (via OCR) into a dedicated incident expense tag (`#incident-tokyo`).
     - Drafts claim emails to the airline via SMTP, ready for one-tap human approval.
  4. Running `!warroom resolve` disbands the room, posts a summary After-Action Report (AAR), logs total financial impact, and creates a knowledge note for future trips.

### 1.5 Spoken Rehearsal Partner & Interactive Voice Sparring
*Low-latency spoken simulation using local ONNX STT + Kokoro-82M TTS.*

- **The Problem:** Preparing for high-stakes conversations (salary negotiation, technical interview, landlord dispute, sales pitch) is hard to do alone.
- **The Concept:** You trigger `!rehearse "Salary negotiation with a frugal engineering manager"` or `!rehearse "System design interview on distributed queues"`.
- **Workflow:**
  - You speak via Telegram/Matrix voice notes or Web Console microphone.
  - Local SenseVoice transcribes speech in realtime; LLM stays strictly in character; local Kokoro-82M speaks back via audio within 1–2 seconds.
  - When you finish (`!rehearse stop`), the bot drops character and provides a detailed analytical debrief:
    - Objection handling score.
    - Conciseness, tone, and filler word usage.
    - Persuasive leverage assessment.
  - Saves the critique and transcripts directly into your Notes as a training log.

### 1.6 Personal Intelligence Dossier Generator
*Instant deep-dive research dossiers on people, companies, or topics before you meet.*

- **The Problem:** You have a meeting with a client, investor, candidate, or doctor in 20 minutes and need a rapid, comprehensive briefing.
- **The Concept:** You send `!dossier "Acme Corp / John Doe"` or forward a calendar invite.
- **Autonomous Pipeline:**
  1. Gathers external intelligence via guarded `web_search` (news, product updates, public bio).
  2. Queries your private second brain for historical touchpoints: past notes, bookmarks, email mentions, or todos referencing the entity.
  3. Synthesizes a structured 1-page **Executive Dossier**:
     - Executive Summary & Key Milestones.
     - Past History / Last Interaction.
     - Potential Landmines / Red Flags.
     - 3 Strategic Talking Points / Questions to Ask.
  4. Delivers as a beautifully formatted PDF/Markdown note, ready for offline mobile reading.

### 1.7 Autonomous "Personal Escrow" & Anti-Procrastination Stakes
*Real skin in the game for non-negotiable habits and deadlines.*

- **The Problem:** It's too easy to snooze a reminder or ignore a todo when there are zero real-world consequences.
- **The Concept:** You set a commitment with stakes:
  ```text
  !stake "Submit tax filing by Sunday 18:00" --penalty "$50 to charity" --proof "receipt_ocr"
  ```
- **Mechanics:**
  - Held in database state with a hard deadline job.
  - To clear the stake, you must provide verifiable proof before the clock expires (e.g. upload a receipt photo that passes OCR validation, or confirm via WebAuthn step-up).
  - If the deadline passes without proof: The bot automatically fires the consequence (e.g. executes an outbound payment webhook, or broadcasts a pre-written embarrassing forfeit message into a shared group chat).

### 1.8 Autonomous "Personal Proxy" / Deal & Bill Negotiator
*A human-in-the-loop autonomous assistant that handles outbound friction.*

- **The Problem:** Dealing with annual subscription renewals, bill increases, and customer service follow-ups is tedious, so people overpay.
- **The Concept:**
  - When `recurring_bills` detects an upcoming renewal or a price jump on an existing bill (e.g. internet service provider rate hike):
  - The agent drafts a polite negotiation or cancellation email quoting competitor rates found via web search.
  - Pauses in `WaitingApproval` with an approval request on Matrix/Web UI:
    > *"Your Internet bill is renewing at $80/mo (up from $60). I've drafted a retention discount request citing Provider B's $55 offer. [Approve & Send via Email] [Dismiss]"*
  - When you tap **Approve**, the bot sends it through the authenticated Email adapter, tracks the reply thread, and notifies you when the vendor responds.

---

## 2. Omnichannel Polish & Multimodal Refinements

Ensure feature parity across chat surfaces while exploiting channel-specific strengths.

### 2.1 Omnichannel Voice Note Ingestion (Speech-to-Text)
- **Workflow:**
  1. Channel adapter downloads the audio stream (Ogg/Opus, M4A, WAV from Telegram, Discord, Matrix, or Email).
  2. Dispatches to local Sherpa-ONNX / SenseVoice STT service in `matrix-bot-inference`.
  3. Transcribed text is fed into the standard `InboundPipeline`, executing either deterministic CLI commands or the direct LLM tool loop.
  4. Returns text confirmation plus action receipts.

### 2.2 Cross-Platform Receipt & Document Ingestion
- **Current State:** Implemented for Matrix chat attachments and Web storage uploads via `media.process_receipt`.
- **Enhancement:**
  - **Telegram:** Handle photo attachments sent in private chats (`sendPhoto` -> download -> storage -> OCR job -> expense recorded).
  - **Discord:** Handle image attachments uploaded in DMs.
  - **Email:** Extract PDF/image invoice attachments from inbound emails, execute OCR, and draft pending expense records.

### 2.3 Discord Interactive ActionRows & Modals
- **Buttons on Responses:** Bring Discord to parity with Telegram's inline keyboards by adding `ActionRow` buttons for quick actions:
  - `/todo` list items with interactive "Done" buttons.
  - Trade confirmations and approval requests with "Approve" (Green) / "Reject" (Red) buttons.
- **Modals:** Pop-up interactive forms for multiline journal entries, habit creation, and expense logging without typing command syntax.

### 2.4 Sunday Night Audio Digest (Kokoro-82M TTS Pipeline)
- **Concept:** Transform `WeeklyBookmarkDigestTask` into a personalized weekly audio podcast.
- **Workflow:**
  1. The agent compiles unread bookmarks, top habit streaks, completed to-dos, and spending summaries.
  2. Synthesizes a structured, conversational 2-to-3 minute audio script.
  3. Invokes local Kokoro-82M inference to generate an Ogg Opus audio clip.
  4. Delivers as a native voice message (`sendVoice` on Telegram, MSC3245 audio bubble on Matrix) with an accompanying markdown outline.

### 2.5 High-Fidelity Responsive HTML Email Digests
- **Concept:** Clean, modern responsive HTML templates for weekly briefs and monthly financial audits delivered over email.
- **Features:** Mobile-optimized tables for expense summaries, visual progress bars for habit completion, and clean plain-text fallback.

---

## 3. Second Brain & Knowledge Graph Evolution

Building upon the implemented pgvector hybrid search and Notes refactor.

### 3.1 Wikilinks (`[[Note Title]]`), Backlinks & Bi-directional Graph
- **Concept:** Obsidian-style bi-directional linking. Writing `[[Machine Learning Architecture]]` inside a note body automatically creates a reference link.
- **Implementation:**
  - In `application/knowledge/markdown.rs`: Extract `\[\[(.*?)\]\]` tokens during note save.
  - A lightweight `note_links (source_note_id, target_title, target_note_id)` table.
  - In Web Console (`modules/knowledge`): A "Backlinks" panel showing notes that reference the current note.
  - In search: Mentioned-but-unlinked notes suggested via `pg_trgm` fuzzy matching.

### 3.2 Spaced Repetition / Flashcard Active Recall
- **Concept:** Turn notes into active recall items.
- **Workflow:** Any note section formatted with `Q: ... / A: ...` or tagged `#review` is added to a SuperMemo-2 / FSRS spaced repetition review queue.
- **Delivery:** The bot sends 1–3 daily flashcards via chat at a scheduled review time, with inline buttons: `Easy` / `Good` / `Hard` / `Again`, updating next review intervals.

---

## 4. Personal Finance & Wealth Management Refinements

Enhance the integer-minor ledger and append-only hash-chained portfolio audit log.

### 4.1 Real-Time Multi-Currency & FX Normalization
- **Concept:** Dynamic currency conversion for international travelers and multi-currency investors.
- **Features:**
  - Scheduled background job fetching daily ECB or open FX rates.
  - View total consolidated net worth in the user's base currency (e.g. USD or EUR) while retaining native currencies (`amount_minor`, `currency`) in transaction records.

### 4.2 CSV & Bank Statement Batch Importer
- **Concept:** Web console drag-and-drop statement importer.
- **Features:**
  - Supports standard bank CSV and OFX/QIF formats.
  - LLM-assisted or regex rule-based category assignment.
  - Pre-commit preview screen displaying proposed transactions with duplicate detection against existing rows.

### 4.3 Dividend & Yield Tracking
- **Features:**
  - Track dividend payments against portfolio holdings.
  - Calculate Yield on Cost (YoC) and annual projected dividend income.

### 4.4 Financial Data Visualizations
- **Features:**
  - Sankey flow diagram for monthly cash flow (Income -> Fixed Bills -> Discretionary Categories -> Savings/Investments).
  - Interactive Net Worth timeline with benchmark overlays (e.g. S&P 500 comparison).

---

## 5. Web Console UI/UX & Mobile Experience

Refine the React 19 + Tailwind v4 web console (`frontend/`) for fast, friction-free daily operation.

### 5.1 Global Command Palette / Omnisearch (`Cmd+K` / `Ctrl+K`)
- **Features:**
  - Instant spotlight search across all modules (notes, todos, bookmarks, transactions, portfolio).
  - Quick-action execution: `Add Expense`, `New Todo`, `Trigger AI Prompt`, `Toggle Theme`.
  - Full keyboard navigation for mouse-free power usage.

### 5.2 WebAuthn Conditional UI (Passkey Autofill)
- **Concept:** Browser-native biometric sign-in directly on page load.
- **Implementation:**
  - Invoke `navigator.credentials.get({ mediation: 'conditional', ... })` on the login screen.
  - When the user focuses the username/passkey field, the browser automatically presents Touch ID / Windows Hello / Face ID without requiring a separate button click.

### 5.3 Progressive Web App (PWA) & Offline Fast-Capture
- **Features:**
  - Web App Manifest and service worker caching for installability on iOS and Android.
  - Offline fast-capture queue: Write a quick todo or note while offline; automatically syncs via the REST API when connectivity returns.

---

## 6. Inference Server & Multimodal Extensions

Leverage the dedicated `matrix-bot-inference` ONNX process for edge machine learning:

### 6.1 Small Vision-Language Models (VLM)
- **Concept:** Run lightweight vision models (e.g., Moondream2 or SmolVLM) in `matrix-bot-inference`.
- **Capabilities:**
  - Describe photos uploaded to chat.
  - Answer questions about charts, whiteboards, diagrams, and documents without routing images to third-party cloud APIs.

### 6.2 Meeting & Long Audio Transcription Pipeline
- **Concept:** Upload long audio recordings (lectures, voice memos, meetings) to generate structured summaries.
- **Workflow:**
  - Decoupled durable job `media.process_audio` chunks long audio, transcribes with Sherpa-ONNX, and uses the LLM to extract key discussion points, decisions, and action items (saved directly as to-dos).

---

## 7. Prioritization Matrix & Horizons

| Horizon | Initiative | User Value | Architectural Complexity |
|---|---|---|---|
| **Horizon 1: High-Impact Daily Polish** | • Passkey Conditional UI (Autofill)<br>• Omnichannel voice note ingestion (STT)<br>• Discord `/todo` action buttons<br>• Global Command Palette (`Cmd+K`)<br>• CSV statement importer | Immediate daily UX smoothness and friction reduction | Low–Medium (reuses existing STT and jobs) |
| **Horizon 2: Active Life Intelligence** | • Sunday Night Audio Digest (Kokoro TTS)<br>• Ambient Life Telemetry & Correlation<br>• Wikilinks & Backlinks Graph<br>• Stale Commitment Resurrector & Serendipity<br>• Multi-currency FX auto-sync | Elevates system from passive tool to active life manager | Medium (new background analytics & link graph) |
| **Horizon 3: Autonomous Agency & Emergent Powers** | • Synthetic Advisory Board & Devil's Advocate<br>• Spoken Rehearsal Partner (Kokoro + STT)<br>• Dynamic Life War Room (ephemeral channels)<br>• Autonomous Proxy Negotiator (Email + Approvals)<br>• Personal Escrow Stakes<br>• Local Small VLM | Delivers truly novel AI superpowers not found in commercial tools | Medium–High (multi-agent prompts, ephemeral rooms, webhooks) |
