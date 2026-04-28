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

## Desktop Startup Acceptance

- Launch the Windows app from the installed bundle.
- Confirm the app opens without an extra black terminal window.
- If starting through PowerShell/CMD for development, the launcher terminal may remain; the app itself should not spawn a separate terminal window.

## Sync Acceptance

- The folder rail shows Inbox. Sent, Archive, Trash, Drafts, and provider-specific folders appear when returned as selectable IMAP folders.
- Selecting each synced folder loads messages for that folder, not only Inbox.
- Folder rows show synced local counts. Folders with unread mail may show `unread/total`; folders without unread mail should show the total count.
- Manual sync can be triggered from the topbar sync button, and folder/message counts refresh after sync.
- Re-running sync does not duplicate messages.
- A folder-level failure appears in sync status without preventing other folders from loading.

## Action Acceptance

- Mark a message read and unread; confirm the state changes locally and after a resync.
- Star and unstar a message; confirm the state changes locally and after a resync.
- Delete a test message only when a synced Trash folder exists; it should move to Trash, not be permanently deleted. If Trash is not synced, create or sync it first, or mark this check not applicable.
- Move and archive are backend-supported but are not part of manual UI acceptance until controls are exposed.
- Compose a test message; it should send directly without a `PENDING ACTIONS` confirmation step.
- Confirm the message is delivered and appears in Sent locally after successful send.
- In `CONFIGURATION` -> `DISPLAY`, enable `ACTIVITY LOG`; recent audit lines should appear in the footer. Disable it again; the footer should disappear and the mail workspace should reclaim the space.

## AI Acceptance

- Configure AI provider settings with an HTTPS OpenAI-compatible endpoint.
- Confirm the AI provider settings live in the unified configuration UI, not on a per-message settings button.
- Analyze one non-sensitive test message.
- Confirm summary, category, priority, todos, and reply draft appear.
- Confirm the summary is concise Simplified Chinese.
- Reopen the same message and confirm saved insight history remains.
- Clear or update AI settings.

## Known Limits

- Attachment files are not downloaded yet; only metadata is indexed.
- Permanent delete is limited to messages already in Trash and should be tested only with disposable mail.
- AI analysis is manual only; API keys and mailbox passwords are stored plaintext in SQLite for this MVP, and only masked AI keys are returned to the UI.
- OAuth is not implemented; use provider app passwords where required.
