# Production Readiness & QA Review Report

**Project:** `matrix-hello-bot` (Rust / Axum / Matrix SDK Backend + React / TypeScript / Vite Frontend)  
**Date:** August 6, 2026  
**Auditor:** Senior Software & Security Engineering Review  

---

## 1. Architecture & Code Quality

### Evaluation & Structural Alignment
The codebase exhibits a clean modular architecture following the plugin pattern required by `AGENTS.md`:
- **Backend**: Plugin system implemented via the `BotModule` trait (`src/modules/mod.rs`), cleanly isolating features (`todo`, `reminder`, `core`). HTTP API endpoints are segregated in `src/api/` with unified error handling (`ApiError`).
- **Frontend**: Plugin registry system (`src/core/registry.ts`) registering feature modules dynamically (`coreModule`, `todoModule`, `reminderModule`).

### Code Smells, Anti-Patterns & Guideline Violations

#### 1. File Line Count Limit Exceeded
- **Violation**: `backend/src/api/auth.rs` contains **664 lines**, violating the strict project rule requiring all source files in `src/` to stay **under 500 lines** (target < 300).
- **Solution**: Decompose `auth.rs` into logical sub-modules:
  - `src/api/auth/types.rs` (`Claims`, `AuthPayload`, `AuthResponse`, `ApiState`)
  - `src/api/auth/extractor.rs` (`FromRequestParts` implementation for `AuthUser`)
  - `src/api/auth/handlers.rs` (`register`, `login`, `get_current_user`, `media_proxy`)

#### 2. Duplicate Domain Normalization Logic
- **Issue**: Matrix homeserver domain clean-up and URL fallback building is duplicated between `bot/client.rs` and `api/auth.rs`.
- **Solution**: Extract homeserver domain resolution into a single utility helper in `src/utils/matrix.rs`.

#### 3. Top-Level Module Side-Effects in Frontend
- **Issue**: Module registration `registry.register(...)` is executed at the file top-level of `App.tsx`. During Vite Hot Module Replacement (HMR), modules trigger repeated warnings (`[ModuleRegistry] Module ... is already registered`).
- **Solution**: Wrap registration inside `useEffect` or an explicit initialization function executed once during boot.

#### 4. Type Safety Erosion (`any` Usage)
- **Issue**: Generic `any` types present in `src/core/types.ts` (`payload: any`, `queryClient: any`) and generic catch blocks in `AuthModal.tsx`.
- **Solution**: Replace `queryClient: any` with `QueryClient` from `@tanstack/react-query` and type event payloads using generics or discriminated unions.

---

## 2. Production Readiness

### 1. Missing Graceful Shutdown Safeguards
- **Location**: `backend/src/main.rs` & `backend/src/api/mod.rs`
- **Problem**: `main.rs` listens for `tokio::signal::ctrl_c()` and exits immediately. Axum HTTP server and Matrix sync loop are spawned as detached background tasks without graceful shutdown channels (`with_graceful_shutdown`). In-flight HTTP requests and pending DB operations are abruptly terminated.
- **Solution**: Implement `axum::serve(listener, app).with_graceful_shutdown(shutdown_signal())` and broadcast a `tokio::sync::watch` shutdown signal to background workers.

### 2. Insecure Default Secrets & Unprotected Registration
- **Location**: `backend/src/config.rs` & `backend/src/api/auth.rs`
- **Problem**: `JWT_SECRET` defaults to `"matrix_bot_secret_jwt_token_key_change_in_production"`. Additionally, `API_REGISTRATION_SECRET` defaults to `None`, allowing unauthenticated public creation of web accounts.
- **Solution**: Fail startup validation (`Config::validate()`) if `JWT_SECRET` matches the hardcoded default when not in debug build modes.

### 3. Non-JSON Gateway Error Crashes in Frontend
- **Location**: `frontend/src/core/apiClient.ts`
- **Problem**: `fetchApi` assumes responses always return valid JSON (`await response.json()`). When reverse proxies (Nginx/Cloudflare) return 502/504 HTML error pages, `response.json()` throws an unhandled `SyntaxError: Unexpected token '<'`.
- **Solution**: Check `response.headers.get('content-type')` or wrap `response.json()` in a `try...catch` block.

---

## 3. Quality Assurance (QA) & Edge Cases

### 1. Fragile Retry Policy in Background Reminder Worker
- **Location**: `backend/src/modules/reminder/scheduler.rs`
- **Problem**: Background loop runs every 5 seconds. If a Matrix room send fails (e.g. transient homeserver latency), `retry_count` is incremented. After 5 attempts (**25 seconds total**), the reminder is permanently marked `status = 'failed'`.
- **Solution**: Implement exponential backoff (e.g., retries at +1m, +5m, +15m) and raise `MAX_RETRIES` to 10.

### 2. Destruction of E2EE Session Crypto Store
- **Location**: `backend/src/bot/client.rs`
- **Problem**: If `client.matrix_auth().session().is_none()`, the code wipes `data/bot_store`. Purging this directory deletes all persisted Megolm session keys, causing permanent loss of historic encrypted message decryption capability.
- **Solution**: Preserve the SQLite crypto store and attempt re-login using the existing store directory.

### 3. Frontend SSE Custom Event Handling Mismatch
- **Location**: `frontend/src/core/useSystemEvents.ts`
- **Problem**: Backend emits typed SSE events (`Event::default().event(event_type)`). Browser `EventSource.onmessage` **only triggers for unnamed default events**. Named events are ignored by `onmessage`.
- **Solution**: Bind listeners for specific named events or stream default message events with type fields inside the JSON body.

### 4. Double Fetching on SSE Event Invalidation
- **Location**: `frontend/src/modules/todo/index.tsx`
- **Problem**: Mutations trigger local query invalidation via `useMutation`, while incoming SSE events also trigger `queryClient.invalidateQueries`. User actions result in duplicate concurrent network requests.
- **Solution**: Debounce or deduplicate invalidation calls.

---

## 4. Security Review

### Vulnerability Matrix

| Vulnerability | Location | Severity | Description |
| :--- | :--- | :---: | :--- |
| **IDOR / Data Isolation Leak** | `api/todos.rs`, `api/reminders.rs` | **CRITICAL** | Endpoints accept `AuthUser` but omit `sender_mxid = user.username` filter in SQL queries. Any user can read/modify/delete all tasks across all users. |
| **Unauthenticated SSE Event Stream** | `api/sse.rs` | **CRITICAL** | `sse_handler` lacks `AuthUser` extractor. Anonymous users can monitor live system notifications. |
| **Credential & Token Exposure in UI** | `AuthModal.tsx`, `VercelShell.tsx` | **HIGH** | `AuthModal` renders raw JWT tokens in plaintext on screen. `VercelShell` appends tokens in avatar query strings (`&token=...`), leaking JWTs in server access logs and `Referer` headers. |
| **Plaintext Access Token Storage** | `bot/client.rs` | **HIGH** | Writes raw Matrix Access Token to `data/matrix_token.json` without strict file permission masks (`0600`). |
| **Permissive Wildcard CORS** | `api/mod.rs` | **HIGH** | `CorsLayer::new().allow_origin(Any)` allows cross-origin exploitation. |

### Code Remediation for Critical IDOR Leak

```rust
// Fix in backend/src/api/todos.rs
pub async fn delete_todo(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let res = sqlx::query("DELETE FROM todos WHERE id = ? AND sender_mxid = ?")
        .bind(id)
        .bind(&user.username)
        .execute(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("Task not found or access denied"));
    }

    Ok(StatusCode::NO_CONTENT)
}
```

---

## 5. Performance Review

### 1. Database Indexing Deficit
- **Location**: `backend/src/db/schema.rs`
- **Problem**: Neither `todos` nor `reminders` tables have indexes. Scheduler queries `WHERE status = 'pending' AND trigger_at <= ?` every 5 seconds, resulting in full table scans.
- **Solution**: Add composite indexes in `schema.rs`:
  ```sql
  CREATE INDEX IF NOT EXISTS idx_reminders_status_trigger ON reminders(status, trigger_at);
  CREATE INDEX IF NOT EXISTS idx_todos_sender_room ON todos(sender_mxid, room_id);
  ```

### 2. Unnecessary Frontend Re-render Pressure
- **Location**: `frontend/src/core/useSystemEvents.ts` & `VercelShell.tsx`
- **Problem**: State updates on every incoming SSE event trigger top-level re-renders of `VercelShell`. Furthermore, `registry.getAll()` is called inside render loops without `useMemo`.

---

## 6. Testing Review

- **Backend**: **32 unit tests pass** (`cargo test`). Good coverage for command parsing, HTML escaping, priority badges, and CRUD repository logic.
- **Gaps**:
  - Missing authorization & multi-tenant security assertion tests.
  - Zero unit, integration, or E2E tests in `frontend/`.

---

## 7. User Experience (UX) Review

- **Modal Accessibility**: `AuthModal.tsx` lacks `role="dialog"`, `aria-modal="true"`, focus lock traps, and `Escape` key listeners.
- **Icon Buttons**: Delete task and modal close buttons lack explicit `aria-label` tags for screen readers.
- **Error Feedback**: Mutation errors in React components fail silently without user-facing toast alerts.

---

## 8. Platform Best Practices

- **Rust**: High adherence to Tokio async patterns and HTML sanitization rules (`html_escape`, `strip_html`).
- **React/TypeScript**: Clean project structure and theme engine. Strict `tsconfig.json` settings enforced.

---

## 9. Release Readiness Report

### Executive Summary

| Dimension | Score (0–100) | Summary Notes |
| :--- | :---: | :--- |
| **Production Readiness** | **62 / 100** | Lacks graceful shutdown, retry backoffs, and rate limiting. |
| **Architecture & Code Quality** | **82 / 100** | Modular architecture; file line limit exceeded in `auth.rs`. |
| **Security** | **48 / 100** | **Critical IDOR flaws**, unauthenticated SSE stream, insecure fallback secrets. |
| **Performance** | **72 / 100** | Missing DB composite indexes on `reminders` & `todos`. |
| **Test Coverage Confidence** | **65 / 100** | 32 backend tests pass; frontend tests missing entirely. |

### Release Recommendation

# 🚧 **Not Yet Production Ready**

---

### Priority Checklist

#### 🚨 Critical (Must Fix Before Release)
- [ ] **Fix REST API IDOR Flaws**: Enforce `sender_mxid` filtering across all `GET`, `PATCH`, and `DELETE` handlers in `api/todos.rs` and `api/reminders.rs`.
- [ ] **Authenticate SSE Stream**: Add `AuthUser` token extractor to `GET /api/v1/events` in `api/sse.rs`.
- [ ] **Enforce Hardened Config Validation**: Reject default `JWT_SECRET` and force `API_REGISTRATION_SECRET` in production in `config.rs`.
- [ ] **Protect E2EE Session Keys**: Remove `remove_dir_all("data/bot_store")` on session expiration in `bot/client.rs`.

#### ⚠️ High (Strongly Recommended Before Release)
- [ ] **Decompose `auth.rs`**: Refactor `api/auth.rs` (664 lines) into smaller files under 500 lines to comply with `AGENTS.md`.
- [ ] **Add Database Indexes**: Add composite indexes for `reminders(status, trigger_at)` and `todos(sender_mxid, room_id)` in `db/schema.rs`.
- [ ] **Implement Graceful Shutdown**: Wire `axum::serve` and Matrix sync loop to cancellation tokens in `main.rs`.
- [ ] **Fix SSE Custom Event Parsing**: Align event listeners in `useSystemEvents.ts`.
- [ ] **Remove Token Query Params in UI**: Remove `&token=...` from avatar image URLs and plaintext token rendering in `AuthModal.tsx`.

#### 🔷 Medium (Next Release Improvements)
- [ ] **Exponential Retry Backoff**: Update `reminder/scheduler.rs` to use exponential backoff instead of fixed 5s retries.
- [ ] **Rate Limiting**: Add rate-limiting middleware (`tower-governor`) to `/auth/login` and `/auth/register`.
- [ ] **Frontend Error Boundaries**: Add React `ErrorBoundary` wrapper in `main.tsx`.

#### 💡 Low (Nice-to-Have)
- [ ] **Restrict Token File Permissions**: Write `data/matrix_token.json` with `0600` permissions.
- [ ] **Modal A11y Polish**: Add keyboard focus trap and ARIA labels to `AuthModal.tsx`.
