# Real Mailbox Acceptance Checklist

Use a disposable or dedicated test mailbox. Do not run destructive checks on a production mailbox.

## Environment

- Node is managed by `fnm`.
- Use `pnpm`, not `npm`.
- IMAP must support TLS.
- SMTP must use implicit TLS on port `465` or STARTTLS on port `587`.

## Setup

1. Run `pnpm install`.
2. Run `pnpm tauri:dev` on a machine with the required Tauri system dependencies.
3. Add the test mailbox through `ACCOUNT LINK`.
4. Click `TEST` and confirm both IMAP and SMTP report OK.
5. Save the account and let the initial sync finish.

## Sync Acceptance

- The folder rail shows Inbox, Sent, Archive, Trash, Drafts, and any provider-specific folders returned by IMAP.
- Selecting each synced folder loads messages for that folder, not only Inbox.
- `SYNC & CONNECTIONS` shows the account as idle after sync.
- Re-running sync does not duplicate messages.
- A folder-level failure appears in sync status without preventing other folders from loading.

## Action Acceptance

- Mark a message read and unread; confirm the state changes locally and after a resync.
- Star and unstar a message; confirm the state changes locally and after a resync.
- Delete a test message; it should move to Trash, not be permanently deleted.
- Move or archive a test message if the target folder exists; it should leave the source folder.
- Compose a test message; it must appear in `PENDING ACTIONS` before sending.
- Confirm the pending send; the message should be delivered.
- Compose another test message and reject it; no message should be delivered.

## Known Limits

- Attachment files are not downloaded yet; only metadata is indexed.
- Permanent delete is intentionally disabled.
- AI local review and remote summaries are not enabled in this phase.
- OAuth is not implemented; use provider app passwords where required.
