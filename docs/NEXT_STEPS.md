# Next Steps

Last updated: 2026-04-27

## Immediate Windows Validation

- [ ] Download the latest GitHub Actions `agentmail-windows-bundles` artifact from `main`.
- [ ] Install and launch the app on Windows.
- [ ] Confirm the app starts without an extra black terminal window.
- [ ] Add or edit a real mailbox through the unified configuration UI and confirm IMAP/SMTP connection tests pass.
- [ ] Close and reopen configuration, then confirm IMAP/SMTP host, port, TLS, email, and password fields are still populated from SQLite.
- [ ] Sync folders and confirm folder counts show synced totals and unread counts.
- [ ] Open several folders and confirm message lists are folder-specific.
- [ ] Configure an HTTPS OpenAI-compatible AI endpoint from the unified configuration UI.
- [ ] Analyze one non-sensitive test message and confirm the summary is concise Simplified Chinese.
- [ ] Compose a message and confirm it appears in `PENDING ACTIONS`.
- [ ] Confirm the pending send and verify delivery.

## High-Priority Product Improvements

- Improve send failure visibility.
  - Show the SMTP error from `confirm failed: ...` in a clearer place near pending actions or the audit log.
  - Acceptance: a failed send tells the user whether the failure is address parsing, authentication, connection, TLS, or provider rejection when available.

- Add project status update discipline.
  - Update `docs/PROJECT_STATUS.md` after meaningful feature work or release validation.
  - Acceptance: a new model session can read the docs and correctly describe current state without chat history.

- Add Windows artifact release notes.
  - Keep the GitHub Actions artifact, and optionally add a release workflow later.
  - Acceptance: users can identify which commit an installer came from.

## Later MVP Extensions

- Attachment file download and safe local opening.
- OAuth or provider-specific account helpers.
- Tray/minimize behavior and Windows notifications.
- Mail rules, labels, templates, contacts, and batch operations.
- Optional AI settings for summary length or output language if users need customization later.
- More robust mailbox provider compatibility testing.

## Handoff Checklist For New Sessions

1. Read `README.md`.
2. Read `docs/PROJECT_STATUS.md`.
3. Read `docs/DECISIONS.md`.
4. Read `docs/NEXT_STEPS.md`.
5. Read `docs/REAL_MAIL_ACCEPTANCE.md`.
6. Run `git status --short`.
7. Run `git log --oneline -5`.
8. Remember: use `pnpm`, do not use `npm` or `npx`, and do not push unless explicitly asked.
