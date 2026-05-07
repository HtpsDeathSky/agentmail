# AgentMail

[![Windows Build](https://github.com/HtpsDeathSky/agentmail/actions/workflows/windows-build.yml/badge.svg)](https://github.com/HtpsDeathSky/agentmail/actions/workflows/windows-build.yml)
[![GitHub Pages](https://github.com/HtpsDeathSky/agentmail/actions/workflows/pages.yml/badge.svg)](https://github.com/HtpsDeathSky/agentmail/actions/workflows/pages.yml)

Windows-first desktop mail client with local sync, direct mail actions, and manual AI analysis.

AgentMail is an MVP built with Tauri v2, Rust, React/Vite, and SQLite. It focuses on practical IMAP/SMTP mail management before broader automation: sync mail locally, search it, act on it, send mail, and optionally ask an OpenAI-compatible remote model to analyze selected messages.

## Features

- IMAP/SMTP account setup with editable connection settings and live tests.
- Gmail internal-test flow through Google sign-in, IMAP XOAUTH2, and SMTP XOAUTH2.
- Local SQLite storage for accounts, folders, messages, sync state, FTS5 search, action audits, and AI insights.
- UID-based folder sync with local folder counts and duplicate-safe refreshes.
- Direct mail actions from the UI, including read state, star, delete, and analyze.
- SMTP sending through `lettre` with implicit TLS on `465` and STARTTLS on `587`.
- Manual AI analysis for selected messages through an HTTPS OpenAI-compatible API.
- Dense desktop UI with dark industrial and archive-beige light themes.
- Windows release bundles built by GitHub Actions.

## Status

AgentMail is an active MVP, not a public-ready mail client.

- Target platform: Windows desktop.
- Gmail OAuth is for internal testing only. Public distribution still requires Google OAuth consent configuration and app verification.
- Mailbox passwords, Gmail OAuth tokens, and AI API keys are stored plaintext in SQLite for this MVP.
- AI analysis is manual only; incoming mail is not analyzed automatically.
- Attachment metadata is indexed, but attachment files are not downloaded yet.
- The app is designed to run without an extra terminal window in Windows GUI builds.

See [docs/PROJECT_STATUS.md](docs/PROJECT_STATUS.md) for the current source of truth.

## Quick Start

This repo uses `fnm` for Node and `pnpm` for package management. Do not use `npm`.

```bash
pnpm install
pnpm dev
```

For desktop development:

```bash
pnpm tauri:dev
```

For a production frontend build:

```bash
pnpm build
```

## Gmail Internal Testing

Create a Google OAuth desktop client, then set the client ID before launching the desktop app:

```bash
export AGENTMAIL_GOOGLE_OAUTH_CLIENT_ID="your-client-id.apps.googleusercontent.com"
pnpm tauri:dev
```

In the app, open `CONFIGURATION` -> `MAIL ACCOUNTS`, choose `Gmail`, and use `SIGN IN WITH GOOGLE`.

## Development

Common checks:

```bash
pnpm test
pnpm build
pnpm rust:check
pnpm rust:test
cargo fmt --all --check
```

Windows bundles are built by `.github/workflows/windows-build.yml` on pushes to `main` and version tags. Linux desktop builds require Tauri/Wry system packages such as WebKitGTK, GTK, `glib-2.0`, and `gio-2.0`; final desktop packaging should be validated on the target Windows toolchain.

## Documentation

- [Project status](docs/PROJECT_STATUS.md)
- [Product and technical decisions](docs/DECISIONS.md)
- [Next steps](docs/NEXT_STEPS.md)
- [Real mailbox acceptance checklist](docs/REAL_MAIL_ACCEPTANCE.md)
- [Original desktop client plan](docs/DESKTOP_AI_MAIL_CLIENT_PLAN.md)

Historical Superpowers outputs live under `docs/superpowers/` for traceability, but the files above are the current handoff source.
