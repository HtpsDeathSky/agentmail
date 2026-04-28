# AgentMail Direct Actions and Console Cleanup Design

## Goal

Remove the user-facing `PENDING ACTIONS` confirmation flow so mail actions execute immediately, remove the always-visible `SYNC & CONNECTIONS` footer panel, and make `AUDIT / ACTIVITY LOG` hidden by default with a setting-controlled display toggle.

## Current Project Context

- AgentMail is a Windows-first Tauri v2 desktop mail client with a Rust workspace backend, React/Vite UI, and SQLite storage.
- The current backend treats SMTP send, permanent delete, forward, batch delete, and batch move as confirmation-required action kinds.
- SMTP send currently writes a `pending_actions` row, writes a queued audit row, creates a local Sent placeholder, and waits for `confirm_action` before calling SMTP.
- The frontend currently renders three footer panels: `SYNC & CONNECTIONS`, `PENDING ACTIONS`, and `AUDIT / ACTIVITY LOG`.
- Sync state is still needed for automatic refresh, account sync, and internal status handling even after the sync footer panel is removed.
- The user approved keeping the `pending_actions` SQLite table for compatibility while stopping all new product behavior from using it.

## Scope

### In Scope

- Stop creating new pending actions for send, permanent delete, and batch actions.
- Execute sends immediately from the compose flow without an extra confirmation step.
- Keep successful sends visible in Sent after completion.
- Keep send failures visible through status text and audit records.
- Remove frontend pending action state, API bindings, handlers, and UI.
- Remove the always-visible `SYNC & CONNECTIONS` footer panel.
- Hide `AUDIT / ACTIVITY LOG` by default.
- Add a persisted setting in the existing `DISPLAY` settings tab to show or hide the activity log.
- Update tests and project handoff docs so they no longer describe pending send confirmation as current behavior.

### Out of Scope

- Dropping the `pending_actions` table from existing SQLite databases.
- Removing historical `pending_actions` store methods if doing so would increase migration risk.
- Redesigning sync, watcher, or automatic refresh internals.
- Adding a destructive-action confirmation modal to replace `PENDING ACTIONS`.
- Changing the existing manual AI analysis behavior.

## Product Behavior

### Direct Send

When the user submits the composer:

1. The UI calls `send_message(draft)`.
2. The backend validates the draft and account configuration.
3. The backend generates a `message_id_header` when the draft does not provide one.
4. The backend sends the message through SMTP immediately.
5. On success, the backend writes an `executed` send audit and makes the message visible in Sent.
6. On failure, the backend writes a `failed` send audit with a sanitized error and returns the error to the UI.
7. The composer stays open on failure and closes only after successful send.

The existing Tauri command can continue returning `String`, but after this change the value should represent the local Sent message id or another successful send identifier, not a pending action id.

The UI status should use direct-send language such as `sent to ...`, not queue or confirmation language.

### Direct Mail Actions

Single-message read, unread, star, unstar, delete, archive, move, Trash permanent delete, and batch variants execute directly through `execute_mail_action`.

Delete keeps the existing normalization rule:

- Deleting outside Trash moves to Trash.
- Deleting from Trash becomes `PermanentDelete`.
- Multi-message delete becomes `BatchDelete`.
- Multi-message move becomes `BatchMove`.

The difference is that normalized destructive and batch actions execute immediately instead of becoming pending actions.

### Footer Console

The normal application layout should no longer reserve space for an operations console. The app uses two rows by default:

- Topbar.
- Main mail workspace.

When the activity log setting is enabled, the app adds a compact footer row for `AUDIT / ACTIVITY LOG` only. The footer does not include `SYNC & CONNECTIONS` or `PENDING ACTIONS`.

### Settings

The existing `DISPLAY` tab gains an `ACTIVITY LOG` toggle. The setting is stored in localStorage under `agentmail-show-activity-log`.

Default value: hidden.

The toggle controls only visibility. Audit data can still be refreshed internally so errors remain available when the user enables the log.

## Architecture

### Backend API

`AppApi::send_message` changes from queue creation to direct execution.

The direct send implementation should preserve these existing safeguards:

- Reject empty recipients before SMTP.
- Normalize or generate `message_id_header`.
- Use `sent_folder_for_local_send` so local Sent display works even before a remote Sent folder is discovered.
- Persist the successful send through a new direct-send store path, not the queued-send helper.
- Attempt Sent reconciliation after successful SMTP send for sync-enabled accounts.
- Avoid leaving a local Sent row when SMTP send fails.
- If SMTP succeeds but local Sent persistence fails, return a local persistence error without automatically retrying SMTP.

`AppApi::execute_mail_action` should still normalize the requested action through the existing request normalization helper, but it should call the execution path directly and return `MailActionResultKind::Executed` on success.

`confirm_action`, `reject_action`, and `list_pending_actions` should no longer be used by the frontend. They may remain as backend compatibility methods during this cleanup if removing them would create unnecessary Tauri command or store churn.

### Store Compatibility

Keep the `pending_actions` table and its migration code for now. Existing databases may already contain the table, and keeping it avoids a risky migration for a product behavior removal.

No new direct-send or direct-action path should call:

- `save_pending_action`
- `save_queued_send_with_placeholder`
- `update_pending_action_status`

The activity audit table remains the product history source.

### Frontend

`ui/src/App.tsx` should remove:

- `PendingMailAction` imports.
- `pendingActions` state.
- `refreshPendingActions`.
- Pending refresh arguments from sync helper request types.
- `handleConfirmPending`.
- `handleRejectPending`.
- `PendingActionQueue`.
- Footer rendering of `SYNC & CONNECTIONS`.

The sync helpers should continue refreshing folders, messages, sync state, and audits. Removing the footer is a display change, not a sync behavior change.

The activity log visibility helper should mirror the existing theme helpers:

- `ACTIVITY_LOG_STORAGE_KEY`
- `readStoredActivityLogVisibility(storage)`
- `writeStoredActivityLogVisibility(storage, visible)`

The root layout can use a class such as `activity-log-visible` when the footer is enabled.

### Browser Demo Backend

The demo backend should match the desktop behavior:

- `send_message` sends immediately in demo terms.
- It creates or updates the Sent message directly.
- It records an `executed` send audit.
- It does not create a pending action.
- Pending action commands can be removed from the TypeScript command map or left unreachable if backend compatibility is temporarily retained only on the Rust side.

## UI Design

Keep the existing dense, utilitarian industrial style. The visible interface should become quieter:

- Topbar keeps sync, compose, and configuration icon buttons.
- Main workspace keeps account rail, folder rail, message list, resizer, message detail, and AI panel.
- Footer is absent by default.
- When enabled, the footer is a single `AUDIT / ACTIVITY LOG` panel with current status and recent audit lines.
- The `DISPLAY` tab contains the existing theme control plus the activity log toggle.

The setting should not use explanatory in-app copy beyond compact labels. The expected label pair is:

- `ACTIVITY LOG`
- `VISIBLE` or `HIDDEN`

## Error Handling

- Direct send failure returns the SMTP or validation error to the composer caller and writes a failed audit.
- Direct send failure must not leave a local Sent message.
- Failed mail actions continue to write failed audits through the existing execution path.
- The UI should show send failure in the main status text and keep the composer open so the user can edit or retry.
- Activity log visibility does not control whether audits are written.

## Testing

### Rust Tests

Update app-api tests to verify:

- `send_message` calls the protocol immediately.
- `send_message` returns a send result without creating a pending action.
- A successful send creates a visible Sent message and an executed audit.
- A failed send writes a failed audit and leaves no local Sent message.
- Trash delete directly executes `PermanentDelete`.
- Batch delete and batch move directly execute after normalization.
- Existing pending-action store round-trip tests can remain as store compatibility tests.

### Frontend Tests

Update UI tests to verify:

- Send status text uses direct-send language.
- Sync helper functions no longer require `refreshPendingActions`.
- Activity log visibility defaults to hidden.
- Activity log visibility can be persisted and read from localStorage.
- The app layout does not reserve the footer row when activity log visibility is hidden.

Update demo API tests to verify:

- Demo send creates a Sent message immediately.
- Demo send no longer exposes a pending action for the sent message.

## Documentation Updates

Update these files after implementation:

- `README.md`
- `docs/PROJECT_STATUS.md`
- `docs/DECISIONS.md`
- `docs/NEXT_STEPS.md`
- `docs/REAL_MAIL_ACCEPTANCE.md`

The documentation should say direct send is current behavior and activity log display is user-controlled from settings.

Historical Superpowers plans and specs should not be rewritten.

## Verification

Run:

```bash
cargo fmt --all --check
cargo test -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api
pnpm test
pnpm build
pnpm rust:check
git diff --check
```

Browser verification should check both default and activity-log-visible layouts at desktop and narrow widths. If Playwright is blocked by a shared browser lock, use the existing `agent-browser` fallback.

## Design Decisions

- Keep the `pending_actions` SQLite table for compatibility, but stop using it in product flows.
- Use the existing audit log as the only durable history surface for mail actions.
- Hide activity log by default to reduce visual noise.
- Keep manual sync controls in the topbar even though the sync footer panel is removed.
- Do not introduce a replacement confirmation dialog.
