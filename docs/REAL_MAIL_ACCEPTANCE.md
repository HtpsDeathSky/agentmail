# Real Mailbox Acceptance Checklist

Use a disposable or dedicated test mailbox. Do not run destructive checks on a production mailbox.

## Environment

- Node is managed by `fnm`.
- Use `pnpm`, not `npm` or `npx`.
- IMAP must support TLS.
- SMTP must use implicit TLS on port `465` or STARTTLS on port `587`.

## Setup

1. Run `pnpm install`.
2. Run `pnpm tauri:dev` on a machine with the required Tauri system dependencies.
3. Add the test mailbox through `ACCOUNT LINK`.
4. Click `TEST` and confirm both IMAP and SMTP report OK.
5. Save the account and let the initial sync finish.

## Sync Acceptance

- The folder rail shows Inbox. Sent, Archive, Trash, Drafts, and provider-specific folders appear when returned as selectable IMAP folders.
- Selecting each synced folder loads messages for that folder, not only Inbox.
- `SYNC & CONNECTIONS` shows the account as idle after sync.
- Re-running sync does not duplicate messages.
- A folder-level failure appears in sync status without preventing other folders from loading.

## Action Acceptance

- Mark a message read and unread; confirm the state changes locally and after a resync.
- Star and unstar a message; confirm the state changes locally and after a resync.
- Delete a test message only when a synced Trash folder exists; it should move to Trash, not be permanently deleted. If Trash is not synced, create or sync it first, or mark this check not applicable.
- Move or archive a test message if the target folder exists; it should leave the source folder. Use a test mailbox/provider with Sent, Archive, Trash, and Drafts for action checks that require those folders.
- Compose a test message; it must appear in `PENDING ACTIONS` before sending.
- Confirm the pending send; the message should be delivered.
- Compose another test message and reject it; no message should be delivered.

## AI Acceptance

- Configure AI provider settings with an OpenAI-compatible endpoint.
- Analyze one non-sensitive test message.
- Confirm summary, category, priority, todos, and reply draft appear.
- Reopen the same message and confirm saved insight history remains.
- Clear or update AI settings.

## Known Limits

- Attachment files are not downloaded yet; only metadata is indexed.
- Permanent delete is intentionally disabled.
- AI analysis is manual only; API keys are stored plaintext in SQLite for this MVP.
- OAuth is not implemented; use provider app passwords where required.
