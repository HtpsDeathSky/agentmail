# AgentMail

Windows-first desktop mail client MVP built with Tauri v2, Rust, React/Vite, SQLite, and Windows Credential Manager.

## Current MVP

- Manual IMAP/SMTP account setup with real connection tests.
- SQLite-backed accounts, folders, messages, sync state, FTS5 search, and action audit log.
- Secret-store abstraction using Windows Credential Manager on Windows and in-memory storage for non-Windows/dev tests.
- Replaceable mail protocol boundary. The desktop backend uses `LiveMailProtocol`; tests and browser demo fallback use `MockMailProtocol`.
- All selectable IMAP folders sync through UID search/fetch with per-folder sync state, MIME parsing,正文文本存储, and attachment metadata indexing without downloading attachment files.
- SMTP send through `lettre`; port `465` uses implicit TLS and port `587` uses STARTTLS.
- Account-level sync locking plus per-folder failure counts and short exponential backoff after sync failures.
- Pending action queue for high-risk actions. SMTP send is queued first and only executes after explicit confirmation.
- Tauri startup triggers background sync for accounts with `sync_enabled=true`.
- Desktop UI shell with account/folder rail, message list, detail pane, compose modal, search, sync controls, and bottom operations console.
- AI modules are intentionally reserved for a later phase.

## Commands

This repo assumes Node is managed by `fnm` and package management is through `pnpm`.

```bash
pnpm install
pnpm dev
pnpm build
cargo test -p mail-core -p mail-store -p secret-store -p mail-protocol -p app-api
cargo fmt --all --check
```

For real mailbox validation, use the checklist in [docs/REAL_MAIL_ACCEPTANCE.md](docs/REAL_MAIL_ACCEPTANCE.md).

For desktop development:

```bash
pnpm tauri:dev
pnpm tauri:build
```

On Linux, Tauri/Wry requires WebKit/GTK development packages such as `glib-2.0`, `gio-2.0`, and WebKitGTK. The target product is Windows, so full Tauri build verification should be run on a Windows builder or a Linux image with Tauri's Linux system dependencies installed.
