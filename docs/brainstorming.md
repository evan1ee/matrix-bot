# Future Feature Enhancements & Refinements

This document captures architectural ideation, potential feature enhancements, and technical refinements for **Matrix Bot**, building upon the completed PostgreSQL 18 layered foundation, omnichannel adapters (Matrix, Discord, Telegram, Email), decoupled object storage, durable job runtime, and autonomous ReAct agent.

---

## 1. Proactive Autonomous Agent Superpowers

With the agent engine (`backend/src/llm/agent/`, native tool calling), durable jobs queue, and passkey step-up approval flow in place, scheduled agent tasks can evolve from simple text notifications into proactive, multi-modal personal assistants.

### 1.1 Sunday Night Audio Digest (Kokoro-82M TTS Pipeline)
- **Concept:** Transform the `WeeklyBookmarkDigestTask` into a personalized weekly audio podcast.
- **Workflow:**
  1. The agent compiles unread bookmarks, top habit streaks, completed to-dos, and spending summaries.
  2. Synthesizes a structured, conversational 2-to-3 minute audio script.
  3. Invokes the local Kokoro-82M inference engine via `interface/jobs/` to generate an Ogg Opus audio clip.
  4. Saves the audio to `StorageService` (`accounts/{id}/audio/{uuid}.opus`).
  5. Delivers the clip as a native voice message (`sendVoice` on Telegram, MSC3245 audio bubble on Matrix) with an accompanying markdown outline.

### 1.2 Proactive Anomaly & Subscription Leakage Watchdog
- **Concept:** An autonomous financial background task checking for budget drift and recurring leaks.
- **Capabilities:**
  - **Price Hike Detection:** Detects unexpected charge increases in recurring bills (e.g. cloud hosting or streaming service fee changes).
  - **Zombie Subscription Audit:** Flags recurring charges for services that have not been logged, mentioned, or referenced in the workspace for over 90 days.
  - **Spending Spike Alerts:** Detects anomalous category spending patterns compared to 3-month rolling averages and suggests adjustments.

### 1.3 Portfolio Drift & Rebalance Recommendation Assistant
- **Concept:** Extend `PortfolioRebalanceAlertTask` with algorithmic rebalancing.
- **Capabilities:**
  - Users define target asset allocation weights (e.g. 70% Equities / 20% Fixed Income / 10% Cash).
  - Daily or weekly ticks calculate drift across holdings.
  - If drift exceeds a user-configured threshold (e.g., 5%), the agent formulates concrete rebalancing trade orders.
  - Intercepted as a sensitive action (`agent_approvals` table), requiring human approval via the Web UI passkey step-up or chat confirmation.

### 1.4 Evening Retrospective & Habit Streak Momentum
- **Concept:** A gentle end-of-day check-in at a configured local time (e.g. 21:30).
- **Capabilities:**
  - Highlights unchecked daily habits with one-click check-ins.
  - Prompts for a 1-sentence mood reflection to record into `life_day_logs`.
  - Celebrates milestones (e.g., 30-day streak achievements).

---

## 2. Omnichannel Polish & Multi-Modal Ingestion

Ensure feature parity across chat surfaces while adhering to channel-specific strengths.

### 2.1 Discord Interactive ActionRows & Modals
- **Buttons on Responses:** Bring Discord to parity with Telegram's inline keyboards by adding `ActionRow` buttons for quick actions:
  - `/todo` list items with interactive "Done" buttons.
  - Trade confirmations and approval requests with "Approve" (Green) / "Reject" (Red) buttons.
- **Modals:** Pop-up interactive forms for multiline journal entries, habit creation, and expense logging without typing command syntax.

### 2.2 Cross-Platform Receipt & Document Ingestion
- **Current State:** Implemented for Matrix chat attachments and Web storage uploads via `media.process_receipt`.
- **Enhancement:**
  - **Telegram:** Handle photo attachments sent in private chats (`sendPhoto` -> download -> storage -> OCR job -> expense recorded).
  - **Discord:** Handle image attachments uploaded in DMs.
  - **Email:** Extract PDF/image invoice attachments from inbound emails, execute OCR, and draft pending expense records.

### 2.3 Omnichannel Voice Note Ingestion (Speech-to-Text)
- **Concept:** Send a voice note in Telegram or Discord DMs when on the go.
- **Workflow:**
  1. Channel adapter downloads the audio stream (Ogg/Opus, M4A, WAV).
  2. Dispatches to local Sherpa-ONNX / SenseVoice STT service in `matrix-bot-inference`.
  3. Transcribed text is fed into the standard `InboundPipeline`, executing either deterministic CLI commands or the direct LLM tool loop.

### 2.4 High-Fidelity Responsive HTML Email Digests
- **Concept:** Clean, modern responsive HTML templates for weekly briefs and monthly financial audits delivered over email.
- **Features:** Mobile-optimized tables for expense summaries, visual progress bars for habit completion, and clean plain-text fallback.

---

## 3. Knowledge Management & Semantic Search (`pgvector`)

Leverage PostgreSQL 18's extensibility to transform the workspace into an intelligent Second Brain without external vector database dependencies.

### 3.1 "Chat with Your Life" (Unified RAG across Domains)
- **Architecture:** Enable `pgvector` extension in PostgreSQL and generate dense embeddings via a small embedding model in `matrix-bot-inference` (or remote API).
- **Vectorized Entities:**
  - Bookmarks (title, description, parsed page content).
  - Life journal entries and daily reflections.
  - Footprints (places visited, reviews, notes).
  - Meeting notes and daily logs.
- **Use Cases:**
  - Natural language cross-module retrieval:
    - *"Where was that coffee shop with great WiFi I visited in Kyoto last year?"*
    - *"What articles did I bookmark about Rust async runtimes and cancellation?"*
    - *"Summarize my thoughts on career goals from last summer's journal entries."*

### 3.2 Automated Bookmark Tagging & Knowledge Graphs
- **Concept:** When a bookmark is saved (`/bookmark add <url>`), automatically extract key concepts, technical tags, and link to related past bookmarks.

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

---

## 5. Web Console UI/UX & Mobile Experience

Refine the React 19 + Tailwind v4 web console (`frontend/`) for fast, friction-free daily operation.

### 5.1 WebAuthn Conditional UI (Passkey Autofill)
- **Concept:** Browser-native biometric sign-in directly on page load.
- **Implementation:**
  - Invoke `navigator.credentials.get({ mediation: 'conditional', ... })` on the login screen.
  - When the user focuses the username/passkey field, the browser automatically presents Touch ID / Windows Hello / Face ID without requiring a separate button click.

### 5.2 Global Command Palette (`Cmd+K` / `Ctrl+K`)
- **Features:**
  - Instant spotlight search across all modules (todos, bookmarks, transactions, portfolio).
  - Quick-action execution: `Add Expense`, `New Todo`, `Trigger AI Prompt`, `Toggle Theme`.
  - Full keyboard navigation for mouse-free power usage.

### 5.3 Progressive Web App (PWA) & Offline Fast-Capture
- **Features:**
  - Web App Manifest and service worker caching for installability on iOS and Android.
  - Offline fast-capture queue: Write a quick todo or note while offline; automatically syncs via the REST API when connectivity returns.

### 5.4 Financial Data Visualizations
- **Features:**
  - Sankey flow diagram for monthly cash flow (Income -> Fixed Bills -> Discretionary Categories -> Savings/Investments).
  - Interactive Net Worth timeline with benchmark overlays (e.g. S&P 500 comparison).

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

## 7. Prioritization & Roadmap Horizons

| Horizon | Initiative | User Value | Architectural Impact |
|---|---|---|---|
| **Horizon 1: Polish & Quick Wins** | • WebAuthn Conditional UI (passkey autofill)<br>• Telegram & Discord photo receipt ingestion<br>• Discord `/todo` action buttons<br>• Global Command Palette (`Cmd+K`) | Immediate daily UX smoothness and friction reduction | Low (re-uses existing jobs and services) |
| **Horizon 2: Intelligent Automation** | • Sunday Night Audio Digest (Kokoro TTS)<br>• Proactive anomaly & subscription watchdog<br>• Multi-currency FX auto-sync<br>• CSV statement importer | Elevates system from passive tool to active life manager | Medium (adds new jobs and domain calculations) |
| **Horizon 3: Strategic Platform Evolution** | • `pgvector` semantic life RAG<br>• PWA offline mobile fast-capture<br>• Local VLM chart & image analysis<br>• Visual cash-flow analytics (Sankey) | Transforms workspace into a complete Second Brain | Medium–High (adds vector extension & new frontend views) |
