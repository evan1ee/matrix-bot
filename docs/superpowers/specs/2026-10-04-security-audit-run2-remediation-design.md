# Security Audit Run-2 Remediation Design

**Date**: 2026-10-04  
**Status**: Proposed  
**Audit Source Reference**: `run-2` findings (`C:/Users/mbmon/security-audit-skill/matrix-hello-bot/run-2/findings.json`)  
**Scope**: Fix the 3 confirmed security findings in `backend` and `inference` crates.

---

## 1. Context & Motivation

During the comprehensive security audit (`run-2`), 3 confirmed security vulnerabilities were identified in `matrix-hello-bot`:

1. **`webhook_sanitize_html_event_handler_bypass` (High Severity)**:
   In `backend/src/domain/webhooks/renderer.rs`, `sanitize_html` checks for event handlers (`on*`) by splitting tag tokens on whitespace, `'/'`, `'>'`, and `'<'`. Quotes (`"` and `'`) and equals signs (`=`) are omitted. Tags with attributes adjacent to quoted values without whitespace (e.g. `<a href="test"onclick="...">`) bypass the check and pass unescaped into Matrix messages, causing stored Cross-Site Scripting (XSS).

2. **`inference.stt.audio-resample-amplification-oom` (High Severity)**:
   In `inference/src/stt/audio.rs`, `decode_generic_audio` extracts `sample_rate` from untrusted container codec parameters without checking for a lower bound (e.g. 8,000 Hz). In `resample_linear`, `ratio = 16000 / from_rate`. When `from_rate = 1`, `ratio = 16,000`. `target_len = samples.len() * 16000`, causing `Vec::with_capacity(target_len)` to attempt multi-gigabyte memory allocations (up to 128 GB for 4 MB audio), triggering an uncatchable heap allocation abort (OOM) that kills the inference daemon.

3. **`agent_cancel_price_alert_unintercepted_deletion` (Medium Severity)**:
   In `backend/src/llm/agent/sandbox.rs`, `DEFAULT_SENSITIVE_TOOLS` lists `"delete_portfolio_price_alert"`, but the tool registered in `backend/src/llm/tools/portfolio/alerts.rs` is `"cancel_portfolio_price_alert"`. Because of this name mismatch, `is_sensitive_tool` returns `false`, allowing autonomous AI agents to permanently delete price alerts without pausing for human-in-the-loop approval.

---

## 2. Technical Design

### 2.1 Webhook HTML Sanitizer (`backend/src/domain/webhooks/renderer.rs`)

#### Invariant
Any HTML tag emitted into Matrix room messages must either have all attributes strictly validated (with no event handlers and no dangerous URI schemes) or be completely escaped (`&lt;...&gt;`).

#### Changes
1. Extend the delimiter closure in `sanitize_html` to include quotes and equals signs:
   ```rust
   let has_dangerous_attr = stripped_whitespace.contains("javascript:")
       || stripped_whitespace.contains("data:")
       || stripped_whitespace.contains("vbscript:")
       || lower_tag.contains("autofocus")
       || lower_tag.contains("formaction")
       || lower_tag
           .split(|c: char| c.is_whitespace() || c == '/' || c == '>' || c == '<' || c == '"' || c == '\'' || c == '=')
           .any(|part| {
               let token = part.trim_start();
               token.starts_with("on")
                   && token.len() > 2
                   && token
                       .chars()
                       .nth(2)
                       .map(|c| c.is_ascii_alphabetic())
                       .unwrap_or(false)
           });
   ```
2. When quotes and equals are delimiters, `<a href="test"onclick="...">` splits into tokens:
   - `"a"`
   - `"href"`
   - `"test"`
   - `"onclick"` -> matches `token.starts_with("on") && token.len() > 2 && token.chars().nth(2) == 'c'` -> `has_dangerous_attr = true`.
   The entire tag is safely escaped as `&lt;a href=&quot;test&quot;onclick=&quot;...&quot;&gt;`.

---

### 2.2 STT Audio Resampling Hardening (`inference/src/stt/audio.rs`)

#### Invariant
The inference service must never allocate unbounded memory based on untrusted container metadata, and must return an explicit `VoiceError::AudioDecode` (HTTP 400) on abnormal audio inputs rather than crashing the process.

#### Changes
1. Define audio frequency and duration constants:
   ```rust
   pub const MIN_SAMPLE_RATE: u32 = 8_000;
   pub const MAX_SAMPLE_RATE: u32 = 192_000;
   pub const MAX_AUDIO_SAMPLES: usize = 16_000 * 300; // 5 minutes at 16kHz mono = 4,800,000 samples
   ```
2. In `decode_generic_audio`:
   Validate `sample_rate`:
   ```rust
   let sample_rate = track.codec_params.sample_rate.unwrap_or(TARGET_SAMPLE_RATE);
   if sample_rate < MIN_SAMPLE_RATE || sample_rate > MAX_SAMPLE_RATE {
       return Err(VoiceError::AudioDecode(format!(
           "Unsupported sample rate: {} Hz (must be between {} and {} Hz)",
           sample_rate, MIN_SAMPLE_RATE, MAX_SAMPLE_RATE
       )));
   }
   ```
3. In `resample_linear`:
   Add safety guards against extreme or invalid `from_rate` and buffer capacity ceiling:
   ```rust
   if from_rate == to_rate || samples.is_empty() {
       return samples.to_vec();
   }
   if from_rate < MIN_SAMPLE_RATE || from_rate > MAX_SAMPLE_RATE {
       return samples.to_vec();
   }
   let ratio = to_rate as f64 / from_rate as f64;
   let target_len = (samples.len() as f64 * ratio).round() as usize;
   if target_len > MAX_AUDIO_SAMPLES {
       return samples.to_vec();
   }
   ```
4. In `decode_audio_to_16k_mono`:
   If decoded samples exceed `MAX_AUDIO_SAMPLES`, return `Err(VoiceError::AudioDecode("Audio duration exceeds 5 minute limit".to_string()))`.

---

### 2.3 Autonomous Agent Sensitive Tools Whitelist (`backend/src/llm/agent/sandbox.rs`)

#### Invariant
All irreversible deletions and financial actions callable by autonomous agents must be classified as sensitive tools, triggering `StepOutcome::ApprovalRequired`.

#### Changes
1. In `backend/src/llm/agent/sandbox.rs`, update `DEFAULT_SENSITIVE_TOOLS`:
   ```rust
   pub const DEFAULT_SENSITIVE_TOOLS: &[&str] = &[
       // Funds & Trades
       "buy_portfolio_holding",
       "sell_portfolio_holding",
       "withdraw_portfolio_cash",
       "deposit_portfolio_cash",
       "pay_recurring_bill",
       // Irreversible deletions
       "delete_finance_entry",
       "delete_todo",
       "delete_recurring_bill",
       "delete_habit",
       "delete_bookmark",
       "delete_footprint",
       "delete_portfolio_price_alert",
       "cancel_portfolio_price_alert",
   ];
   ```
2. Both `"cancel_portfolio_price_alert"` (the actual registered tool name) and `"delete_portfolio_price_alert"` (for backwards/alias safety) are included.

---

## 3. Verification Strategy

1. **Unit Tests**:
   - `backend/src/domain/webhooks/renderer.rs`:
     - Test unspaced adjacent attributes (`href="test"onclick="..."`) are escaped.
     - Test single-quote adjacent attributes (`href='test'onfocus='...'`) are escaped.
     - Test equal-adjacent attributes (`href="test"=onclick="..."`) are escaped.
     - Test normal tags (`<a href="https://example.com">...</a>`, `<b>...</b>`) remain unescaped.
   - `inference/src/stt/audio.rs`:
     - Test `resample_linear` with `from_rate = 1` and `from_rate = 0` does not panic or allocate out of bounds.
     - Test `decode_generic_audio` on WAV files with `sample_rate = 1` or `sample_rate = 4000` returns `VoiceError::AudioDecode`.
     - Test normal 8 kHz, 16 kHz, 44.1 kHz, 48 kHz WAV audio decodes and resamples accurately.
   - `backend/src/llm/agent/sandbox.rs`:
     - Test `is_sensitive_tool("cancel_portfolio_price_alert", None)` returns `true`.
     - Test `is_sensitive_tool("delete_portfolio_price_alert", None)` returns `true`.
2. **Quality Gates**:
   - `cargo check --workspace --all-targets`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace --all-targets`
   - `cargo test -p matrix-bot --test architecture`
