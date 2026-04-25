# AgentMail AI Mail MVP Design

## Goal

Build a Windows-only desktop AI mail management MVP on the existing Rust + Tauri + React codebase. The MVP keeps the current mail sync, search, read, send, and pending-action foundations, then adds a user-triggered remote AI analysis flow for selected mail.

## Current Project Context

- The project already has a Rust workspace, Tauri v2 shell, React/Vite UI, SQLite store, IMAP/SMTP protocol boundary, Windows Credential Manager support for mail passwords, and a pending action queue.
- Existing verification passes with `pnpm build` and `cargo test -p mail-core -p mail-store -p secret-store -p mail-protocol -p app-api`.
- The current AI surface is only reserved: `ai_insights` and `ai_audits` tables exist, and the UI still shows `AI PIPELINE RESERVED`.
- The workspace is not a git repository, so design and implementation work cannot be committed unless git is initialized or the project is moved into a git repo.

## Scope

### In Scope

- User manually selects a message and clicks an AI analysis button.
- Backend loads the selected message from local SQLite.
- Backend calls a remote OpenAI-compatible API.
- Backend saves the model result locally in SQLite.
- UI shows the latest and historical AI results in the right-side message detail panel.
- AI API settings are managed inside the app and stored in SQLite.
- AI API key is stored in SQLite as plaintext for MVP simplicity.

### Out of Scope

- Full Foxmail clone.
- Local sensitivity audit or local model review.
- Automatic analysis of all incoming mail.
- AI-initiated send, delete, forward, move, archive, or batch actions.
- Contacts, rules, calendar, templates, OAuth, tray notifications, attachment downloads, and multi-window behavior.
- Encrypting AI API keys with DPAPI, Windows Credential Manager, or a master password.

## Product Behavior

The user flow is:

1. User opens a synced message.
2. User clicks `AI ANALYZE` in the AI panel.
3. UI sends `run_ai_analysis(message_id)` to the Tauri backend.
4. Backend reads AI settings and the selected message.
5. Backend sends subject, sender, recipients, cc, received time, body preview, body text, and attachment metadata to the remote model.
6. Backend parses the model response into structured fields.
7. Backend saves the result to local SQLite.
8. UI refreshes and displays summary, category, priority, todos, reply draft, model, created time, and raw status.

Manual click is treated as user consent for remote upload. The MVP does not block or redact based on sensitivity.

## Architecture

Add one new Rust crate:

- `crates/ai-remote`: remote AI provider abstraction and OpenAI-compatible HTTP implementation.

Extend existing crates:

- `mail-core`: AI DTOs used by store, API, Tauri commands, and UI type mirrors.
- `mail-store`: AI settings and AI insight repository methods.
- `app-api`: orchestration commands for saving settings, masking settings for UI, running analysis, and listing insights.
- `src-tauri`: expose the new commands.
- `ui`: replace the placeholder AI panel with settings and analysis UI.

Do not put AI provider logic directly in `app-api`. `app-api` should only coordinate store reads, provider calls, and store writes.

## Data Model

Add or normalize these SQLite tables:

### `ai_settings`

- `id TEXT PRIMARY KEY`
- `provider_name TEXT NOT NULL`
- `base_url TEXT NOT NULL`
- `model TEXT NOT NULL`
- `api_key TEXT NOT NULL`
- `enabled INTEGER NOT NULL`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`

MVP stores `api_key` as plaintext. The backend must not include the full key in logs, audit records, or UI read responses.

### `ai_insights`

Use the existing table name but standardize payload structure:

- `id TEXT PRIMARY KEY`
- `message_id TEXT NOT NULL`
- `provider_name TEXT NOT NULL`
- `model TEXT NOT NULL`
- `summary TEXT NOT NULL`
- `category TEXT NOT NULL`
- `priority TEXT NOT NULL`
- `todos_json TEXT NOT NULL`
- `reply_draft TEXT NOT NULL`
- `raw_json TEXT NOT NULL`
- `created_at TEXT NOT NULL`

If retaining the current generic `kind` and `payload_json` shape is faster, it is acceptable for MVP, but the stored payload must contain the fields above.

## Backend API

Expose these app-level methods and Tauri commands:

- `get_ai_settings() -> AiSettingsView`
- `save_ai_settings(request: SaveAiSettingsRequest) -> AiSettingsView`
- `clear_ai_settings() -> ()`
- `run_ai_analysis(message_id: String) -> AiInsight`
- `list_ai_insights(message_id: String) -> Vec<AiInsight>`

`AiSettingsView` returns provider name, base URL, model, enabled flag, and masked API key only. It must not return plaintext `api_key`.

`SaveAiSettingsRequest` accepts plaintext `api_key`. If the user leaves the API key field blank while updating other settings, preserve the existing key.

## Remote AI Contract

The first provider is OpenAI-compatible chat completions:

- Endpoint: `{base_url}/chat/completions`
- Auth: `Authorization: Bearer {api_key}`
- Model: from SQLite settings
- Response format: request JSON output from the model

The system prompt should require a compact JSON object:

```json
{
  "summary": "string",
  "category": "string",
  "priority": "low|normal|high|urgent",
  "todos": ["string"],
  "reply_draft": "string"
}
```

If parsing fails, store no insight and return a clear error to UI. The UI should keep the message usable and show the failure in the AI panel.

## UI Design

Keep the existing dense three-pane desktop layout and Soviet hacker industrial tone.

Replace the current AI placeholder with:

- Settings status row: provider, model, configured/missing key.
- `AI ANALYZE` button for the selected message.
- Latest result block: summary, priority, category, todos, reply draft.
- History list for prior analyses on the same message.
- Compact error/status display for missing settings, remote failure, and parse failure.

Keep the UI utilitarian: compact panels, monospace labels, low-saturation colors, hard borders, no marketing layout.

## Error Handling

- Missing AI settings: `run_ai_analysis` returns an invalid request error.
- Missing message: return not found.
- Missing body: use body preview and metadata.
- Remote HTTP failure: return provider error and do not save a partial insight.
- Invalid model JSON: return parse error and do not save a partial insight.
- API key must be redacted in displayed errors if a provider echoes request metadata.

## Testing

Rust tests:

- AI settings save, update, clear, and masked read.
- API key remains available internally but is not returned by `get_ai_settings`.
- AI insight save/list round trip.
- `run_ai_analysis` with mock provider stores a structured result.
- Missing settings blocks analysis.
- Provider failure does not save an insight.
- Invalid JSON response does not save an insight.

Frontend tests:

- AI panel shows missing settings state.
- AI panel triggers analysis and renders result.
- AI panel shows errors without breaking message viewing.

Verification commands:

```bash
pnpm build
cargo test -p mail-core -p mail-store -p secret-store -p mail-protocol -p app-api
```

All Node commands must use `pnpm`. Do not use `npm` or `npx`.

## Design Decisions

- MVP is manual AI analysis only.
- Manual click is user consent for remote upload.
- No local sensitivity auditing in MVP.
- No automatic AI actions in MVP.
- AI API key is stored in SQLite as plaintext because the target is local single-user desktop use and easy management is preferred.
- Full key must still be excluded from UI read responses, logs, and audit records.
- OpenAI-compatible provider is the first provider because it works with many remote model vendors.

## Review Notes

- This design intentionally keeps the first AI slice small enough to implement without restructuring the whole mail engine.
- `app-api`, `mail-store`, `mail-protocol`, and `ui/src/App.tsx` are already large. Implementation should add focused modules instead of increasing file size unnecessarily.
- Tauri CSP is currently disabled and should be tightened before packaging, but it is not required to complete this AI MVP slice.
