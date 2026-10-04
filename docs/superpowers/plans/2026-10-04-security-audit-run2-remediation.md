# Implementation Plan: Security Audit Run-2 Remediation

**Design:** [`docs/superpowers/specs/2026-10-04-security-audit-run2-remediation-design.md`](file:///d:/01_Data/code/matrix-hello-bot/docs/superpowers/specs/2026-10-04-security-audit-run2-remediation-design.md)  
**Status:** Completed  
**Target Branch:** `fix/security-audit-run2-remediation`

---

## User Review Required

> [!IMPORTANT]
> This plan remediates all three confirmed security vulnerabilities discovered in `run-2`:
> 1. Webhook HTML sanitizer delimiter bypass (`webhook_sanitize_html_event_handler_bypass`)
> 2. Audio resampling memory amplification daemon crash (`inference.stt.audio-resample-amplification-oom`)
> 3. Autonomous agent price alert deletion unintercepted bypass (`agent_cancel_price_alert_unintercepted_deletion`)
> 
> No architectural changes or breaking REST API / OpenAPI contract modifications are made.

---

## Proposed Changes

### Task 1: Fix Webhook HTML Sanitizer Attribute Parsing Edge Case
- **File**: `backend/src/domain/webhooks/renderer.rs`
- **Actions**:
  1. Update `sanitize_html`:
     Extend delimiter split to include quotes (`"`, `'`) and `=`:
     ```rust
     lower_tag
         .split(|c: char| c.is_whitespace() || c == '/' || c == '>' || c == '<' || c == '"' || c == '\'' || c == '=')
     ```
  2. Add unit tests in `mod tests` of `renderer.rs`:
     - Test unspaced attribute `<a href="test"onclick="alert(1)">Click</a>` -> escaped.
     - Test single-quote unspaced `<a href='test'onfocus='alert(1)'>Click</a>` -> escaped.
     - Test equal-separated `<a href="test"=onclick="alert(1)">Click</a>` -> escaped.
     - Test legitimate tags `<a href="https://example.com">Link</a>`, `<b>bold</b>`, `<font color="red">text</font>` -> preserved unescaped.
- **Verification**: `cargo test -p matrix-bot --lib domain::webhooks::renderer::tests`

---

### Task 2: Fix STT Audio Resample Amplification and Add Buffer Bounds
- **File**: `inference/src/stt/audio.rs`
- **Actions**:
  1. Add constants:
     - `pub const MIN_SAMPLE_RATE: u32 = 8_000;`
     - `pub const MAX_SAMPLE_RATE: u32 = 192_000;`
     - `pub const MAX_AUDIO_SAMPLES: usize = 16_000 * 300;` (5 minutes at 16kHz mono = 4,800,000 samples)
  2. In `decode_generic_audio`:
     Validate `sample_rate`: if `sample_rate < MIN_SAMPLE_RATE || sample_rate > MAX_SAMPLE_RATE`, return `Err(VoiceError::AudioDecode(...))`.
  3. In `resample_linear`:
     Add guards:
     - If `from_rate < MIN_SAMPLE_RATE || from_rate > MAX_SAMPLE_RATE`, return `samples.to_vec()`.
     - If `target_len > MAX_AUDIO_SAMPLES`, return `samples.to_vec()`.
  4. In `decode_audio_to_16k_mono`:
     Ensure total decoded samples do not exceed `MAX_AUDIO_SAMPLES`.
  5. Add unit tests in `inference/src/stt/audio.rs`:
     - Test `resample_linear` with `from_rate = 1` and `from_rate = 0` does not abort or allocate excessive capacity.
     - Test `decode_audio_to_16k_mono` with WAV having `sample_rate = 1` or `sample_rate = 4000` returns `VoiceError::AudioDecode`.
     - Test `decode_audio_to_16k_mono` with valid WAV at 8 kHz and 16 kHz decodes correctly.
- **Verification**: `cargo test -p matrix-bot-inference --lib stt::audio::tests`

---

### Task 3: Fix Autonomous Agent Sensitive Tool Check Naming Mismatch
- **File**: `backend/src/llm/agent/sandbox.rs`
- **Actions**:
  1. In `DEFAULT_SENSITIVE_TOOLS`:
     Add `"cancel_portfolio_price_alert"` alongside `"delete_portfolio_price_alert"`.
  2. Add unit tests in `mod tests` of `sandbox.rs`:
     - Assert `is_sensitive_tool("cancel_portfolio_price_alert", None)` is `true`.
     - Assert `is_sensitive_tool("delete_portfolio_price_alert", None)` is `true`.
- **Verification**: `cargo test -p matrix-bot --lib llm::agent::sandbox::tests`

---

### Task 4: Full Workspace Verification & Documentation
- **Actions**:
  1. Run `cargo check --workspace --all-targets`
  2. Run `cargo clippy --workspace --all-targets -- -D warnings`
  3. Run `cargo test --workspace --all-targets`
  4. Run `cargo test -p matrix-bot --test architecture`
  5. Update `docs/superpowers/HANDOFF.md` with remediation status and rulings.

---

## Verification Plan

### Automated Tests
```powershell
# 1. Individual module unit tests
cargo test -p matrix-bot --lib domain::webhooks::renderer::tests
cargo test -p matrix-bot-inference --lib stt::audio::tests
cargo test -p matrix-bot --lib llm::agent::sandbox::tests

# 2. Architecture & lint compliance
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p matrix-bot --test architecture

# 3. Full test suite
cargo test --workspace --all-targets
```
