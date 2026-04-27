# Project Decisions

Last updated: 2026-04-27

## Product Decisions

- Build only a Windows desktop application for the MVP.
- Use Foxmail as a functional reference, but do not attempt a full Foxmail clone in the MVP.
- Keep the first version focused on mail sync, search, reading, sending, and manual AI analysis.
- AI analysis is manual: the user selects a message and clicks the AI analysis button.
- Manual AI click is treated as user consent to send that message content to the configured remote model.
- Do not implement local sensitivity auditing in the MVP.
- AI output should be concise Simplified Chinese by default.

## Technical Decisions

- Architecture: Rust workspace + Tauri v2 + React/Vite + SQLite.
- Node is managed by `fnm`.
- Use `pnpm` only; do not use `npm` or `npx`.
- Frontend calls Tauri commands only; it must not read SQLite directly.
- Store AI API keys plaintext in SQLite for this MVP.
- Do not return the full AI API key to the UI after saving; return only a mask.
- Mailbox passwords go through `secret-store`; Windows uses Windows Credential Manager.
- SMTP send must enter `PENDING ACTIONS` first and execute only after explicit confirmation.
- Windows app should not show an extra terminal window in dev/debug/release app binaries.
- GitHub Actions builds Windows release bundles from `main`.

## Repository Workflow Decisions

- Without explicit user instruction, do not run `git push`.
- Local commits are acceptable when they preserve handoff state and the user has requested implementation.
- Do not commit `.codex`.
- Keep project documentation under `docs/`, except root `README.md`.
- `docs/superpowers/*` is Superpowers plugin history/process output. Do not reorganize it as current project memory.
- Current handoff memory lives in:
  - `docs/PROJECT_STATUS.md`
  - `docs/DECISIONS.md`
  - `docs/NEXT_STEPS.md`
  - `docs/REAL_MAIL_ACCEPTANCE.md`

## Historical Documents

- `docs/DESKTOP_AI_MAIL_CLIENT_PLAN.md` is an early planning document and may contain superseded decisions.
- Historical plan items that are not current MVP decisions include local AI guard, blocking remote AI by sensitivity level, and avoiding plaintext AI API keys.
