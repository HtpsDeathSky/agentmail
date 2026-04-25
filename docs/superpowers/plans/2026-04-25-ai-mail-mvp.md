# AgentMail AI Mail MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a manual remote AI analysis flow for selected email messages, storing AI settings and analysis results in local SQLite.

**Architecture:** Keep the existing mail engine intact. Add shared AI DTOs in `mail-core`, SQLite repository support in `mail-store`, a new `ai-remote` crate for OpenAI-compatible calls, app-level orchestration in `app-api`, Tauri commands in `src-tauri`, and a compact AI panel in the existing React UI.

**Tech Stack:** Rust workspace, Tauri v2, React/Vite, SQLite via rusqlite, OpenAI-compatible HTTP via reqwest, `pnpm` only for Node commands.

---

## File Structure

- Modify `Cargo.toml`: add `crates/ai-remote` to workspace members and add workspace HTTP dependencies.
- Create `crates/ai-remote/Cargo.toml` and `crates/ai-remote/src/lib.rs`: AI provider trait, mock provider for tests, OpenAI-compatible implementation.
- Modify `crates/mail-core/src/lib.rs`: shared AI settings, input, payload, insight, priority, and request/view DTOs.
- Modify `crates/mail-store/src/lib.rs`: `ai_settings` migration plus AI settings and insight repository methods. Reuse the existing generic `ai_insights(kind, payload_json)` table with `kind = 'mail_analysis'`.
- Modify `crates/app-api/Cargo.toml` and `crates/app-api/src/lib.rs`: inject AI provider, save/read settings, run analysis, list insights.
- Modify `src-tauri/Cargo.toml` and `src-tauri/src/main.rs`: expose new AI commands.
- Modify `ui/src/api.ts`, `ui/src/App.tsx`, `ui/src/data/demoBackend.ts`, and `ui/src/styles/app.css`: front-end types, demo behavior, AI settings and analysis panel.
- Modify `README.md` and `docs/REAL_MAIL_ACCEPTANCE.md`: document AI MVP commands and manual acceptance checks.

## Task 1: Shared AI Domain Types

**Files:**
- Modify: `crates/mail-core/src/lib.rs`
- Test: `crates/app-api/src/lib.rs` tests in later tasks will consume these types.

- [ ] **Step 1: Add AI DTOs to `mail-core`**

Append these public types after `MessageFetchRequest`:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiPriority {
    Low,
    Normal,
    High,
    Urgent,
}

impl Default for AiPriority {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiSettings {
    pub id: String,
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub enabled: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiSettingsView {
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
    pub enabled: bool,
    pub api_key_mask: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SaveAiSettingsRequest {
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiAnalysisInput {
    pub message_id: String,
    pub subject: String,
    pub sender: String,
    pub recipients: Vec<String>,
    pub cc: Vec<String>,
    pub received_at: Timestamp,
    pub body_preview: String,
    pub body: Option<String>,
    pub attachments: Vec<AttachmentRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiInsightPayload {
    pub summary: String,
    pub category: String,
    pub priority: AiPriority,
    pub todos: Vec<String>,
    pub reply_draft: String,
    pub raw_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiInsight {
    pub id: String,
    pub message_id: String,
    pub provider_name: String,
    pub model: String,
    pub summary: String,
    pub category: String,
    pub priority: AiPriority,
    pub todos: Vec<String>,
    pub reply_draft: String,
    pub raw_json: String,
    pub created_at: Timestamp,
}
```

- [ ] **Step 2: Run the focused Rust check**

Run:

```bash
cargo check -p mail-core
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/mail-core/src/lib.rs
git commit -m "feat: add ai domain types"
```

## Task 2: SQLite AI Settings and Insight Store

**Files:**
- Modify: `crates/mail-store/src/lib.rs`
- Test: `crates/mail-store/src/lib.rs`

- [ ] **Step 1: Write failing store tests**

Add tests under the existing `#[cfg(test)] mod tests` in `crates/mail-store/src/lib.rs`:

```rust
#[test]
fn ai_settings_round_trip_and_clear() {
    let store = MailStore::memory().unwrap();
    let now = now_rfc3339();
    let settings = AiSettings {
        id: "default".to_string(),
        provider_name: "openai-compatible".to_string(),
        base_url: "https://api.example.com/v1".to_string(),
        model: "mail-model".to_string(),
        api_key: "sk-local-test".to_string(),
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };

    store.save_ai_settings(&settings).unwrap();
    assert_eq!(store.get_ai_settings().unwrap().unwrap().api_key, "sk-local-test");

    store.clear_ai_settings().unwrap();
    assert!(store.get_ai_settings().unwrap().is_none());
}

#[test]
fn ai_insights_round_trip_by_message() {
    let store = MailStore::memory().unwrap();
    let insight = AiInsight {
        id: "insight-1".to_string(),
        message_id: "message-1".to_string(),
        provider_name: "openai-compatible".to_string(),
        model: "mail-model".to_string(),
        summary: "Short summary".to_string(),
        category: "operations".to_string(),
        priority: AiPriority::High,
        todos: vec!["Reply before 18:00".to_string()],
        reply_draft: "Acknowledged.".to_string(),
        raw_json: "{\"summary\":\"Short summary\"}".to_string(),
        created_at: now_rfc3339(),
    };

    store.save_ai_insight(&insight).unwrap();
    let rows = store.list_ai_insights("message-1").unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].summary, "Short summary");
    assert_eq!(rows[0].priority, AiPriority::High);
    assert_eq!(rows[0].todos, vec!["Reply before 18:00".to_string()]);
    assert!(store.list_ai_insights("other-message").unwrap().is_empty());
}
```

Also import the new types in the test module:

```rust
use mail_core::{AiInsight, AiPriority, AiSettings};
```

- [ ] **Step 2: Run failing tests**

Run:

```bash
cargo test -p mail-store ai_settings_round_trip_and_clear ai_insights_round_trip_by_message
```

Expected: FAIL because `save_ai_settings`, `get_ai_settings`, `clear_ai_settings`, `save_ai_insight`, and `list_ai_insights` do not exist.

- [ ] **Step 3: Add the `ai_settings` migration**

In `MailStore::migrate`, add this SQL before the existing `ai_insights` table:

```sql
CREATE TABLE IF NOT EXISTS ai_settings (
  id TEXT PRIMARY KEY,
  provider_name TEXT NOT NULL,
  base_url TEXT NOT NULL,
  model TEXT NOT NULL,
  api_key TEXT NOT NULL,
  enabled INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

Keep the existing `ai_insights` table. Do not add a destructive migration.

- [ ] **Step 4: Implement AI repository methods**

Add public methods to `impl MailStore`:

```rust
pub fn save_ai_settings(&self, settings: &AiSettings) -> StoreResult<()>;
pub fn get_ai_settings(&self) -> StoreResult<Option<AiSettings>>;
pub fn clear_ai_settings(&self) -> StoreResult<()>;
pub fn save_ai_insight(&self, insight: &AiInsight) -> StoreResult<()>;
pub fn list_ai_insights(&self, message_id: &str) -> StoreResult<Vec<AiInsight>>;
```

Implementation requirements:

- `save_ai_settings` upserts by `id`.
- `get_ai_settings` returns the row with `id = 'default'`.
- `clear_ai_settings` deletes all rows from `ai_settings`.
- `save_ai_insight` writes `kind = 'mail_analysis'` into the existing `ai_insights` table and stores the full `AiInsight` as `payload_json`.
- `list_ai_insights` filters by `message_id` and `kind = 'mail_analysis'`, ordered by `created_at DESC`.

- [ ] **Step 5: Add helper row mapping**

Add private helpers near existing row mapping functions:

```rust
fn ai_settings_from_row(row: &Row<'_>) -> rusqlite::Result<AiSettings> {
    Ok(AiSettings {
        id: row.get(0)?,
        provider_name: row.get(1)?,
        base_url: row.get(2)?,
        model: row.get(3)?,
        api_key: row.get(4)?,
        enabled: row.get::<_, bool>(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn ai_insight_from_row(row: &Row<'_>) -> rusqlite::Result<AiInsight> {
    let payload_json: String = row.get(0)?;
    serde_json::from_str::<AiInsight>(&payload_json).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(err),
        )
    })
}
```

- [ ] **Step 6: Run store tests**

Run:

```bash
cargo test -p mail-store
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/mail-store/src/lib.rs
git commit -m "feat: persist ai settings and insights"
```

## Task 3: AI Remote Provider Crate

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/ai-remote/Cargo.toml`
- Create: `crates/ai-remote/src/lib.rs`
- Test: `crates/ai-remote/src/lib.rs`

- [ ] **Step 1: Add workspace member and dependencies**

In root `Cargo.toml`, add:

```toml
members = [
  "crates/mail-core",
  "crates/mail-store",
  "crates/secret-store",
  "crates/mail-protocol",
  "crates/ai-remote",
  "crates/app-api",
  "src-tauri",
]
```

Add workspace dependencies:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

- [ ] **Step 2: Create crate manifest**

Create `crates/ai-remote/Cargo.toml`:

```toml
[package]
name = "ai-remote"
version = "0.1.0"
edition.workspace = true

[dependencies]
async-trait.workspace = true
mail-core = { path = "../mail-core" }
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
```

- [ ] **Step 3: Write provider tests**

Create `crates/ai-remote/src/lib.rs` with the error enum, trait, mock provider, and tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::{
        now_rfc3339, AiAnalysisInput, AiInsightPayload, AiPriority, AiSettings, AttachmentRef,
    };

    fn settings() -> AiSettings {
        let now = now_rfc3339();
        AiSettings {
            id: "default".to_string(),
            provider_name: "openai-compatible".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            model: "mail-model".to_string(),
            api_key: "sk-local-test".to_string(),
            enabled: true,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    fn input() -> AiAnalysisInput {
        AiAnalysisInput {
            message_id: "message-1".to_string(),
            subject: "Release train".to_string(),
            sender: "ops@example.com".to_string(),
            recipients: vec!["me@example.com".to_string()],
            cc: Vec::new(),
            received_at: now_rfc3339(),
            body_preview: "Build passed smoke tests.".to_string(),
            body: Some("Build passed smoke tests. Reply before 18:00.".to_string()),
            attachments: vec![AttachmentRef {
                id: "att-1".to_string(),
                message_id: "message-1".to_string(),
                filename: "report.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size_bytes: 512,
                local_path: None,
            }],
        }
    }

    #[tokio::test]
    async fn mock_provider_returns_configured_payload() {
        let provider = MockAiProvider::new(AiInsightPayload {
            summary: "Summary".to_string(),
            category: "ops".to_string(),
            priority: AiPriority::High,
            todos: vec!["Reply".to_string()],
            reply_draft: "Thanks.".to_string(),
            raw_json: "{}".to_string(),
        });

        let result = provider.analyze_mail(&settings(), &input()).await.unwrap();
        assert_eq!(result.summary, "Summary");
        assert_eq!(result.priority, AiPriority::High);
    }

    #[test]
    fn parses_model_json_payload() {
        let parsed = parse_model_json(
            r#"{"summary":"S","category":"ops","priority":"urgent","todos":["A"],"reply_draft":"R"}"#,
        )
        .unwrap();

        assert_eq!(parsed.priority, AiPriority::Urgent);
        assert_eq!(parsed.todos, vec!["A".to_string()]);
    }
}
```

- [ ] **Step 4: Implement trait and mock provider**

The public API must include:

```rust
use async_trait::async_trait;
use mail_core::{AiAnalysisInput, AiInsightPayload, AiSettings};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AiRemoteError {
    #[error("ai settings are disabled")]
    Disabled,
    #[error("ai settings are incomplete: {0}")]
    InvalidSettings(String),
    #[error("remote ai request failed: {0}")]
    Request(String),
    #[error("remote ai response parse failed: {0}")]
    Parse(String),
}

pub type AiRemoteResult<T> = Result<T, AiRemoteError>;

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn analyze_mail(
        &self,
        settings: &AiSettings,
        input: &AiAnalysisInput,
    ) -> AiRemoteResult<AiInsightPayload>;
}

#[derive(Clone)]
pub struct MockAiProvider {
    payload: Option<AiInsightPayload>,
    error_message: Option<String>,
}

#[derive(Clone, Default)]
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
}
```

`MockAiProvider` returns a configured `AiInsightPayload` or configured error for app-api tests.

- [ ] **Step 5: Implement OpenAI-compatible HTTP provider**

`OpenAiCompatibleProvider` requirements:

- Validate `settings.enabled`, non-empty `base_url`, `model`, and `api_key`.
- POST to `{base_url.trim_end_matches('/')}/chat/completions`.
- Send `Authorization: Bearer {api_key}`.
- Request model JSON output in the system prompt.
- Include subject, sender, recipients, cc, received_at, body_preview, body, and attachment metadata in the user prompt.
- Parse the first `choices[0].message.content` string as JSON into `AiInsightPayload`.
- Store the raw content string as `raw_json`.
- On errors, redact the API key if it appears in an error string.

- [ ] **Step 6: Run crate tests**

Run:

```bash
cargo test -p ai-remote
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/ai-remote
git commit -m "feat: add ai remote provider"
```

## Task 4: App API AI Orchestration

**Files:**
- Modify: `crates/app-api/Cargo.toml`
- Modify: `crates/app-api/src/lib.rs`
- Test: `crates/app-api/src/lib.rs`

- [ ] **Step 1: Add dependency**

In `crates/app-api/Cargo.toml`, add:

```toml
ai-remote = { path = "../ai-remote" }
serde_json.workspace = true
```

- [ ] **Step 2: Write app-api tests**

Add tests named:

- `ai_settings_are_saved_and_masked`
- `run_ai_analysis_requires_settings`
- `run_ai_analysis_stores_provider_result`
- `provider_failure_does_not_store_insight`

Required assertions:

- `save_ai_settings` returns `api_key_mask = Some("sk-...test")` for `sk-local-test`.
- `get_ai_settings` never returns plaintext `api_key`.
- Missing settings returns `ApiError::InvalidRequest`.
- A mock provider result creates one stored insight for the message.
- Provider failure leaves `list_ai_insights(message_id)` empty.

- [ ] **Step 3: Extend `AppApi` state**

Add:

```rust
use ai_remote::{AiProvider, OpenAiCompatibleProvider};
```

Extend `AppApi`:

```rust
ai_provider: Arc<dyn AiProvider>,
```

Keep the existing constructor stable:

```rust
pub fn new(
    store: MailStore,
    secrets: Arc<dyn SecretStore>,
    protocol: Arc<dyn MailProtocol>,
) -> Self {
    Self::new_with_ai_provider(
        store,
        secrets,
        protocol,
        Arc::new(OpenAiCompatibleProvider::default()),
    )
}

pub fn new_with_ai_provider(
    store: MailStore,
    secrets: Arc<dyn SecretStore>,
    protocol: Arc<dyn MailProtocol>,
    ai_provider: Arc<dyn AiProvider>,
) -> Self {
    Self {
        store,
        secrets,
        protocol,
        ai_provider,
        sync_locks: Arc::new(Mutex::new(HashSet::new())),
    }
}
```

- [ ] **Step 4: Add API methods**

Implement:

```rust
pub fn get_ai_settings(&self) -> ApiResult<Option<AiSettingsView>>;

pub fn save_ai_settings(
    &self,
    request: SaveAiSettingsRequest,
) -> ApiResult<AiSettingsView>;

pub fn clear_ai_settings(&self) -> ApiResult<()>;

pub async fn run_ai_analysis(&self, message_id: String) -> ApiResult<AiInsight>;

pub fn list_ai_insights(&self, message_id: String) -> ApiResult<Vec<AiInsight>>;
```

Implementation rules:

- Use singleton settings id `default`.
- Validate non-empty provider name, base URL, model, and API key when there is no existing key.
- Preserve the existing key when `SaveAiSettingsRequest.api_key` is `None`.
- Mask keys with this behavior:
  - length <= 4: `"****"`
  - length > 4: first 3 chars + `...` + last 4 chars
- Build `AiAnalysisInput` from the selected `MailMessage`.
- Create `AiInsight` in app-api using `new_id()` and `now_rfc3339()`.
- Save only after provider success.

- [ ] **Step 5: Extend error conversion**

Add `AiRemoteError` to `ApiError`:

```rust
#[error(transparent)]
AiRemote(#[from] ai_remote::AiRemoteError),
```

Ensure errors do not include plaintext API key.

- [ ] **Step 6: Run app-api tests**

Run:

```bash
cargo test -p app-api
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/app-api/Cargo.toml crates/app-api/src/lib.rs Cargo.lock
git commit -m "feat: orchestrate ai mail analysis"
```

## Task 5: Tauri Command Surface

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Add imported DTOs**

Import the new mail-core types:

```rust
use mail_core::{AiInsight, AiSettingsView, SaveAiSettingsRequest};
```

- [ ] **Step 2: Add Tauri commands**

Add:

```rust
#[tauri::command]
fn get_ai_settings(state: State<'_, ApiState>) -> Result<Option<AiSettingsView>, String> {
    state.api.get_ai_settings().map_err(to_error)
}

#[tauri::command]
fn save_ai_settings(
    state: State<'_, ApiState>,
    request: SaveAiSettingsRequest,
) -> Result<AiSettingsView, String> {
    state.api.save_ai_settings(request).map_err(to_error)
}

#[tauri::command]
fn clear_ai_settings(state: State<'_, ApiState>) -> Result<(), String> {
    state.api.clear_ai_settings().map_err(to_error)
}

#[tauri::command]
async fn run_ai_analysis(
    state: State<'_, ApiState>,
    message_id: String,
) -> Result<AiInsight, String> {
    state.api.run_ai_analysis(message_id).await.map_err(to_error)
}

#[tauri::command]
fn list_ai_insights(
    state: State<'_, ApiState>,
    message_id: String,
) -> Result<Vec<AiInsight>, String> {
    state.api.list_ai_insights(message_id).map_err(to_error)
}
```

- [ ] **Step 3: Register commands**

Add all five command names to `tauri::generate_handler!`.

- [ ] **Step 4: Run desktop crate check**

Run:

```bash
cargo check -p agentmail-app
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/main.rs Cargo.lock
git commit -m "feat: expose ai tauri commands"
```

## Task 6: Frontend API Types and Demo Backend

**Files:**
- Modify: `ui/src/api.ts`
- Modify: `ui/src/data/demoBackend.ts`

- [ ] **Step 1: Add TypeScript AI types**

In `ui/src/api.ts`, add:

```ts
export type AiPriority = "low" | "normal" | "high" | "urgent";

export interface AiSettingsView {
  provider_name: string;
  base_url: string;
  model: string;
  enabled: boolean;
  api_key_mask?: string | null;
}

export interface SaveAiSettingsRequest {
  provider_name: string;
  base_url: string;
  model: string;
  api_key?: string | null;
  enabled: boolean;
}

export interface AiInsight {
  id: string;
  message_id: string;
  provider_name: string;
  model: string;
  summary: string;
  category: string;
  priority: AiPriority;
  todos: string[];
  reply_draft: string;
  raw_json: string;
  created_at: Timestamp;
}
```

- [ ] **Step 2: Extend `CommandMap` and `api`**

Add:

```ts
get_ai_settings: AiSettingsView | null;
save_ai_settings: AiSettingsView;
clear_ai_settings: null;
run_ai_analysis: AiInsight;
list_ai_insights: AiInsight[];
```

Add API wrappers:

```ts
getAiSettings: () => call("get_ai_settings"),
saveAiSettings: (request: SaveAiSettingsRequest) => call("save_ai_settings", { request }),
clearAiSettings: () => call("clear_ai_settings"),
runAiAnalysis: (messageId: string) => call("run_ai_analysis", { messageId }),
listAiInsights: (messageId: string) => call("list_ai_insights", { messageId })
```

- [ ] **Step 3: Extend demo backend**

In `ui/src/data/demoBackend.ts`, add in-memory variables:

```ts
let aiSettings: AiSettingsView | null = {
  provider_name: "openai-compatible",
  base_url: "https://api.example.com/v1",
  model: "demo-mail-model",
  enabled: true,
  api_key_mask: "sk-...demo"
};
let aiInsights: AiInsight[] = [];
```

Add switch cases for the five new commands. `run_ai_analysis` should create a deterministic demo insight from the selected message subject and body preview.

- [ ] **Step 4: Run frontend type/build check**

Run:

```bash
pnpm build
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/api.ts ui/src/data/demoBackend.ts
git commit -m "feat: add frontend ai api bindings"
```

## Task 7: React AI Panel and Settings UI

**Files:**
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/styles/app.css`

- [ ] **Step 1: Add UI state**

In `App`, add state for:

```ts
const [aiSettings, setAiSettings] = useState<AiSettingsView | null>(null);
const [aiInsights, setAiInsights] = useState<AiInsight[]>([]);
const [isAnalyzing, setAnalyzing] = useState(false);
const [aiStatus, setAiStatus] = useState("ai link idle");
const [isAiSettingsOpen, setAiSettingsOpen] = useState(false);
```

Import `AiInsight`, `AiSettingsView`, and `SaveAiSettingsRequest`.

- [ ] **Step 2: Add refresh callbacks**

Add:

```ts
const refreshAiSettings = useCallback(async () => {
  setAiSettings(await api.getAiSettings());
}, []);

const refreshAiInsights = useCallback(async (messageId: string | null) => {
  if (!messageId) {
    setAiInsights([]);
    return;
  }
  setAiInsights(await api.listAiInsights(messageId));
}, []);
```

Call `refreshAiSettings()` during startup and `refreshAiInsights(selectedMessageId)` whenever the selected message changes.

- [ ] **Step 3: Add analyze handler**

Add:

```ts
const handleAnalyze = useCallback(async () => {
  if (!selectedMessageId) return;
  setAnalyzing(true);
  setAiStatus("ai analysis running");
  try {
    await api.runAiAnalysis(selectedMessageId);
    await refreshAiInsights(selectedMessageId);
    setAiStatus("ai analysis complete");
  } catch (error) {
    setAiStatus(`ai analysis failed: ${String(error)}`);
  } finally {
    setAnalyzing(false);
  }
}, [refreshAiInsights, selectedMessageId]);
```

- [ ] **Step 4: Replace placeholder with `AiPanel`**

Replace the existing `<aside className="ai-placeholder">` block with:

```tsx
<AiPanel
  settings={aiSettings}
  insights={aiInsights}
  status={aiStatus}
  isAnalyzing={isAnalyzing}
  onAnalyze={handleAnalyze}
  onOpenSettings={() => setAiSettingsOpen(true)}
/>
```

- [ ] **Step 5: Add `AiPanel` component**

Implement a compact panel that shows:

- provider/model/key status row
- `AI ANALYZE` button
- latest insight summary/category/priority/todos/reply draft
- history rows
- status text

Use existing lucide icons. Keep button labels short and monospace where labels are operational.

- [ ] **Step 6: Add `AiSettingsModal` component**

The modal fields:

- Provider name, default `openai-compatible`
- Base URL, default `https://api.openai.com/v1`
- Model
- API key input
- Enabled checkbox

Submit calls `api.saveAiSettings`. If API key input is empty, send `api_key: null` so backend preserves the existing key.

- [ ] **Step 7: Add CSS**

Add classes:

```css
.ai-panel
.ai-panel header
.ai-status-grid
.ai-actions
.ai-result
.ai-todo-list
.ai-history
.ai-settings-form
```

Style with the existing hard-bordered, low-saturation industrial theme. Do not introduce gradients or marketing-style cards.

- [ ] **Step 8: Run frontend build**

Run:

```bash
pnpm build
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add ui/src/App.tsx ui/src/styles/app.css
git commit -m "feat: add ai analysis panel"
```

## Task 8: Documentation and Acceptance

**Files:**
- Modify: `README.md`
- Modify: `docs/REAL_MAIL_ACCEPTANCE.md`

- [ ] **Step 1: Update README**

Add AI MVP notes:

- AI analysis is manual only.
- Remote provider must be OpenAI-compatible.
- AI API key is stored plaintext in SQLite for MVP.
- Full API key is not returned to the UI after save.
- Use `pnpm`, not `npm` or `npx`.

- [ ] **Step 2: Update real mailbox checklist**

Add checks:

- Configure AI provider settings.
- Analyze one non-sensitive test message.
- Confirm summary/category/priority/todos/reply draft appear.
- Reopen the same message and confirm saved insight history remains.
- Clear or update AI settings.

- [ ] **Step 3: Commit**

```bash
git add README.md docs/REAL_MAIL_ACCEPTANCE.md
git commit -m "docs: document ai mvp acceptance"
```

## Task 9: Full Verification

**Files:**
- No source edits unless verification reveals a defect.

- [ ] **Step 1: Run Rust tests**

Run:

```bash
cargo test -p mail-core -p mail-store -p secret-store -p mail-protocol -p ai-remote -p app-api
```

Expected: PASS.

- [ ] **Step 2: Run frontend build**

Run:

```bash
pnpm build
```

Expected: PASS.

- [ ] **Step 3: Run workspace check**

Run:

```bash
cargo check -p agentmail-app
```

Expected: PASS.

- [ ] **Step 4: Inspect git status**

Run:

```bash
git status --short
```

Expected: only unrelated pre-existing untracked files should remain. No modified implementation files should be left uncommitted.

## Implementation Notes

- Do not use `npm` or `npx`; all Node commands must use `pnpm`.
- Do not add local sensitivity scanning in this MVP.
- Do not auto-run AI analysis on sync.
- Do not allow AI to execute mail actions.
- Do not return plaintext API key from `get_ai_settings`.
- Do not log the API key.
- Keep `app-api` as orchestration only; provider HTTP code belongs in `ai-remote`.
- Preserve current mail sync, search, send, and pending action behavior.
