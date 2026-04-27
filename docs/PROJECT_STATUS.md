# Project Status

Last updated: 2026-04-27

## Current Source of Truth

- Repository: `HtpsDeathSky/agentmail`
- Branch: `main`
- Current HEAD: `e1816ed docs: add project handoff memory`
- Current working tree: unified configuration and SQLite-only mailbox password storage changes are in progress and not yet committed.
- Local working tree note: `.codex` may appear as an untracked local directory; do not treat it as project state and do not commit it.
- Use this file with `docs/DECISIONS.md`, `docs/NEXT_STEPS.md`, and `docs/REAL_MAIL_ACCEPTANCE.md` as the cross-session handoff memory.
- `docs/superpowers/*` contains historical Superpowers plugin outputs. It is useful for traceability but is not the current status source.

## Product Goal

AgentMail is a Windows-only desktop AI mail client MVP inspired by Foxmail. It focuses on practical mail management plus manual AI analysis, not a full Foxmail clone.

The user flow is:

1. Add an IMAP/SMTP mailbox manually.
2. Sync selectable folders and messages into local SQLite.
3. Read, search, and manage messages in the desktop app.
4. Compose mail; sending enters `PENDING ACTIONS` and executes only after explicit confirmation.
5. Manually analyze a selected message through an HTTPS OpenAI-compatible remote API.
6. Save AI summary, category, priority, todos, and reply draft locally.

## Implemented MVP Capabilities

- Windows desktop shell with Tauri v2, Rust workspace, React/Vite UI, and SQLite.
- Unified configuration UI for editable IMAP/SMTP account setup with connection testing.
- SQLite-backed accounts, folders, messages, sync state, FTS5 search, pending actions, action audits, AI settings, and AI insights.
- IMAP/SMTP account passwords are stored plaintext in SQLite for this MVP.
- Live IMAP folder discovery and per-folder UID-based message sync.
- MIME parsing with body text storage and attachment metadata indexing; attachment files are not downloaded.
- Folder counts are refreshed from locally stored messages after sync and confirmed actions.
- SMTP sending uses `lettre`; port `465` uses implicit TLS and port `587` uses STARTTLS.
- SMTP send is queued first and only executes after user confirmation in `PENDING ACTIONS`.
- Tauri startup triggers background sync for accounts with `sync_enabled=true`.
- Manual remote AI analysis for the selected message only.
- AI provider is OpenAI-compatible over HTTPS.
- All mail accounts share one global AI model configuration.
- AI summary prompt now asks for concise Simplified Chinese output.
- Windows app builds as a GUI subsystem so no extra terminal window is shown by the app itself.
- GitHub Actions builds Windows release bundles and uploads `agentmail-windows-bundles`.

## Current Limits

- MVP does not implement local AI sensitivity auditing.
- MVP does not automatically analyze incoming mail.
- AI API keys are stored plaintext in SQLite by product decision.
- Mailbox passwords are stored plaintext in SQLite. Windows Credential Manager is no longer used.
- OAuth is not implemented; use provider app passwords where required.
- Attachment files are not downloaded yet.
- Permanent delete is intentionally disabled.
- Move/archive backend support exists, but not all controls are exposed in the current UI.
- No tray mode, notifications, contacts, rules, calendar, templates, or multi-window support yet.

## Recent Verification

For the current working tree, the following local checks were run on 2026-04-27:

- `cargo fmt --all --check`
- `pnpm test`
- `pnpm rust:test`
- `pnpm rust:check`
- `pnpm build`
- Browser smoke check through Playwright against `pnpm dev -- --host 127.0.0.1`

Environment caveat:

- `cargo check -p agentmail-app` on this Linux environment is blocked by missing Tauri Linux system libraries such as `pango`, `cairo`, `gdk-3.0`, `libsoup-3.0`, `glib-2.0`, and JavaScriptCoreGTK.
- `cargo check -p agentmail-app --target x86_64-pc-windows-msvc` requires MSVC tools such as `lib.exe`.
- Full Windows packaging should be validated through GitHub Actions on `windows-latest`.
