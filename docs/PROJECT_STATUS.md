# Project Status

Last updated: 2026-04-28

## Current Source of Truth

- Repository: `HtpsDeathSky/agentmail`
- Branch: `main`
- Direct-actions cleanup and consistency-first automatic sync are implemented on `main`; this status includes direct-send result hardening, browser-demo parity fixes, and the 2026-04-29 shift away from active QQ IDLE watcher sync.
- Current working tree: run `git status --short` in the active checkout before starting new work.
- Local working tree note: `.codex` may appear as an untracked local directory; do not treat it as project state and do not commit it.
- Use this file with `docs/DECISIONS.md`, `docs/NEXT_STEPS.md`, and `docs/REAL_MAIL_ACCEPTANCE.md` as the cross-session handoff memory.
- `docs/superpowers/*` contains historical Superpowers plugin outputs. It is useful for traceability but is not the current status source.

## Product Goal

AgentMail is a Windows-only desktop AI mail client MVP inspired by Foxmail. It focuses on practical mail management plus manual AI analysis, not a full Foxmail clone.

The user flow is:

1. Add an IMAP/SMTP mailbox manually.
2. Sync selectable folders and messages into local SQLite.
3. Read, search, and manage messages in the desktop app.
4. Compose mail; sending executes directly through SMTP and records an audit entry.
5. Manually analyze a selected message through an HTTPS OpenAI-compatible remote API.
6. Save AI summary, category, priority, todos, and reply draft locally.

## Implemented MVP Capabilities

- Windows desktop shell with Tauri v2, Rust workspace, React/Vite UI, and SQLite.
- Unified configuration UI for editable IMAP/SMTP account setup with connection testing.
- SQLite-backed accounts, folders, messages, sync state, FTS5 search, action audits, AI settings, and AI insights.
- IMAP/SMTP account passwords are stored plaintext in SQLite for this MVP.
- Live IMAP folder discovery and per-folder UID-based message sync.
- Automatic refresh is consistency-sync driven for all enabled accounts; startup, foreground resume, a 120-second running-app interval, account save, and manual sync all use account-level sync.
- IMAP IDLE watcher code remains isolated for future providers, but it is not part of the active sync path in this stage.
- Folder create/delete discovery is not realtime in this stage; startup, account save, interval, foreground, or manual sync refreshes the folder list.
- MIME parsing with body text storage and attachment metadata indexing; attachment files are not downloaded.
- Folder counts are refreshed from locally stored messages after sync and direct actions.
- SMTP sending uses `lettre`; port `465` uses implicit TLS and port `587` uses STARTTLS.
- SMTP send executes directly from the compose flow and records executed or failed audit entries.
- `AUDIT / ACTIVITY LOG` is hidden by default and can be shown from the `DISPLAY` settings tab.
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
- Permanent delete is limited to messages already in Trash and should be tested only with disposable mail.
- Move/archive backend support exists, but not all controls are exposed in the current UI.
- No tray mode, notifications, contacts, rules, calendar, templates, or multi-window support yet.

## Recent Verification

For the consistency-sync architecture work, the following checks should be run on 2026-04-29:

- `cargo fmt --all --check`
- `git diff --check`
- `cargo clippy -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api --all-targets -- -D warnings`
- `cargo test -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api`
- `pnpm test`
- `pnpm build`
- `pnpm rust:check`
- `rg -n "startAccountWatchers|watcher start failed|WATCH_DIAGNOSTIC_EVENT" ui/src` returned no matches.

Environment caveat:

- `cargo check -p agentmail-app` on this Linux environment is blocked by missing Tauri Linux system libraries such as `atk`, `gio-2.0`, `gobject-2.0`, `javascriptcoregtk-4.1`, `gdk-pixbuf-2.0`, `glib-2.0`, `gdk-3.0`, `cairo`, `libsoup-3.0`, and `pango`.
- `cargo check -p agentmail-app --target x86_64-pc-windows-msvc` requires MSVC tools such as `lib.exe`.
- Full Windows packaging should be validated through GitHub Actions on `windows-latest`.
