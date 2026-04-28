# QQ Dynamic IMAP IDLE Sync Design

## Goal

Make QQ Mail automatic sync event-driven through IMAP IDLE, without background polling, while keeping the existing manual sync button and adding a visible waiting state for manual sync.

## Current Context

AgentMail already has a partial IMAP IDLE path:

- `mail-protocol` checks IMAP `CAPABILITY` for `IDLE`, selects a folder, and waits for `IdleResponse::NewData`.
- `src-tauri/src/main.rs` owns a watcher registry keyed by `account_id\0folder_id`.
- Watcher-triggered sync emits `agentmail-mail-sync` to the React UI.
- The frontend still has a 30-second `runAutomaticAccountSync` interval fallback.

The next stage should make the QQ Mail path IDLE-first and remove automatic polling. It should also decouple watcher orchestration from `src-tauri/src/main.rs` so the sync behavior is testable and easier to evolve.

## Scope

### In Scope

- QQ Mail only.
- Use the remote QQ folder list as the dynamic source of folders to watch.
- Watch every selectable folder returned by IMAP `LIST`, after filtering `\NoSelect`.
- Keep the topbar manual sync button.
- Add a loading spinner or waiting ring to the manual sync button while manual sync runs.
- Remove the frontend 30-second automatic sync interval.
- Add folder-level sync so an IDLE event syncs only the changed folder.
- Move watcher orchestration out of `src-tauri/src/main.rs` into a focused watcher service module.
- Preserve the existing `agentmail-mail-sync` event bridge to refresh UI state after backend updates.
- Treat one folder watcher failure as folder-scoped. Other folder watchers for the same QQ account must keep running.

### Out Of Scope

- Gmail support.
- Provider-specific Gmail label or deduplication work.
- Foxmail-style menu actions for "sync current account" versus "sync all accounts".
- Background discovery of newly created or deleted remote folders.
- Connection-count limits or folder-count caps.
- Attachment download and cache behavior.
- General large-file refactors unrelated to sync/watch behavior.

## Business Rules

1. A saved QQ account with `sync_enabled=true` performs an initial account sync before watchers start.
2. After initial sync succeeds, the app starts IDLE watchers for every selectable folder currently known for that QQ account.
3. Automatic sync is event-driven only. The app must not use a background timer to periodically fetch mail.
4. IDLE timeout or disconnect recovery may recreate the IDLE connection. This is connection maintenance, not polling.
5. When any watched folder receives an IDLE change event, the app syncs that folder and emits `agentmail-mail-sync`.
6. The UI refreshes folders, messages, sync state, and audits after a sync event for the selected account.
7. Manual sync remains available. Clicking the manual sync button runs an explicit account sync, refreshes UI state, and restarts or reconciles watchers after the sync.
8. While manual sync is running, the button shows a waiting ring and is disabled to prevent duplicate manual syncs.
9. Newly added or deleted folders are not discovered in the background in this stage. They are discovered through startup sync, account save sync, or manual sync.

## Architecture

### Protocol Layer

`mail-protocol` remains responsible for raw IMAP operations:

- `fetch_folders(settings, account)` lists remote folders and filters `\NoSelect`.
- `watch_folder_until_change(settings, account, folder)` performs `CAPABILITY`, validates `IDLE`, selects the folder, enters IDLE, and returns `Changed` or `Timeout`.
- Add a folder-level fetch path that syncs a single `MailFolder` without requiring a full account sync.

No QQ-specific folder names should be hardcoded in the watcher planner. QQ folder selection comes from the actual remote folder list.

### App API Layer

`app-api` exposes:

- `sync_account(account_id)` for startup, account save, and manual sync.
- `sync_folder(account_id, folder_id)` for IDLE-triggered updates.
- Existing folder ownership, sync state, and backoff behavior should be reused.

`sync_folder` must:

- Validate that the folder belongs to the account.
- Respect `sync_enabled=false`.
- Fetch messages for that one folder using existing incremental UID state.
- Save messages and refresh folder counts.
- Update folder-level sync state.
- Avoid mutating sibling folders.

### Watch Service

Create a focused watcher module at `src-tauri/src/watchers.rs` for this stage. It owns Tauri-facing orchestration and keeps `main.rs` small.

Suggested model:

```text
AccountWatchService
  account_id
  registry: HashMap<folder_id, FolderWatchHandle>

FolderWatchHandle
  folder_id
  folder_path
  status
  last_event_at
  last_sync_at
  last_error
  retry_count
```

The initial implementation does not need to persist every watch status to SQLite. It should maintain runtime status and update existing sync state enough for UI feedback.

### Watch Planning

The desired watcher set is:

```text
desired_folders = list_folders(account_id)
  .filter(folder is selectable)
```

Because `mail-protocol::fetch_folders` already filters `\NoSelect`, the stored folder list can be used as the desired watcher source.

Reconcile behavior:

- If a desired folder has no watcher, start one.
- If a watcher already exists for a desired folder, keep it.
- If a watcher exists for a folder no longer in the desired list, stop it during explicit reconcile.
- Folder additions and deletions are only discovered when the desired folder list is refreshed by startup sync, account save sync, or manual sync.

### Folder Watch Loop

Each folder watcher loop:

1. Confirms the account is still `sync_enabled`.
2. Calls `watch_folder_until_change(account_id, folder_id)`.
3. On `Changed`, calls `sync_folder(account_id, folder_id)`.
4. Emits `agentmail-mail-sync` with the account id, folder id, reason `watch_changed`, and a short message.
5. Re-enters IDLE for the same folder.
6. On `Timeout`, re-enters IDLE without syncing.
7. On unsupported IDLE or authentication failure, records folder-level error/backoff and exits or retries based on the error class.
8. On transient network errors, sleeps with backoff and then retries the same folder.

`ApiError::SyncAlreadyRunning(_)` remains nonfatal. The watcher should keep running.

## Frontend Behavior

### Remove Automatic Polling

Remove the `AUTO_SYNC_INTERVAL_MS` interval and the frontend automatic sync effect. This removes background polling.

Remove `runAutomaticAccountSync` from the UI flow and tests unless TypeScript compilation proves another non-polling code path still imports it.

### Keep Manual Sync

Manual sync remains a user-triggered command.

Add UI state:

```text
isManualSyncing: boolean
```

Manual sync button behavior:

- Disabled while `isManualSyncing=true`.
- Shows a spinner or waiting ring next to the sync icon.
- Status text changes to `sync running` while active.
- On completion, status shows the existing sync summary.
- On failure, status shows the error and the spinner stops.

Foxmail-style split actions for current account versus all accounts are deferred.

### Sync Event Refresh

Keep `agentmail-mail-sync` listener behavior. After an event for the selected account:

- Refresh folders.
- Refresh messages for the current view.
- Refresh sync state.
- Refresh audits.

If the UI later gains a unified all-mail view, the same event should refresh that local query. That unified view is not required in this stage.

## Data Flow

### Startup

```text
Tauri setup
-> list sync-enabled accounts
-> sync_account(account_id)
-> emit startup sync event
-> start/reconcile QQ account watchers from stored folder list
```

### Manual Sync

```text
User clicks sync
-> isManualSyncing = true
-> sync_account(account_id)
-> refresh visible UI state
-> start/reconcile QQ account watchers
-> isManualSyncing = false
```

### IDLE Change

```text
Folder watcher receives Changed
-> sync_folder(account_id, folder_id)
-> emit agentmail-mail-sync
-> UI refreshes selected account state
-> watcher re-enters IDLE
```

## Error Handling

- IDLE unsupported: mark the folder or account as unsupported for realtime sync. Do not start polling.
- Authentication failure: stop watcher and surface the error through sync state/status.
- Network failure: retry with backoff for that folder only.
- Sync already running: treat as nonfatal; continue watching.
- Single folder failure: do not stop other folder watchers.
- Account disabled: stop watchers for that account.

## Testing Strategy

### Rust Unit Tests

- Watch planner starts watchers for every folder returned by the stored folder list.
- Watch planner does not hardcode Inbox/Sent/Trash/Junk.
- Watcher reconcile starts missing folder watchers and keeps existing watchers.
- `sync_folder` validates folder ownership.
- `sync_folder` updates only the target folder.
- `SyncAlreadyRunning` does not terminate a folder watcher.

### Frontend Tests

- Manual sync button enters a loading state while sync is pending.
- Manual sync button is disabled while pending.
- Manual sync completion clears the loading state and refreshes state.
- Automatic interval sync is removed from `App`.

### Manual Acceptance

Use a real QQ mailbox:

1. Configure QQ IMAP/SMTP account with an authorization code.
2. Start AgentMail.
3. Confirm all existing selectable QQ folders are loaded.
4. Send a test message to QQ Inbox and confirm it appears without pressing manual sync.
5. Send or move a message so it lands in another QQ folder and confirm that folder updates without pressing manual sync.
6. Click manual sync and confirm the button shows a waiting ring until completion.
7. Confirm no background 30-second automatic sync status appears.

## Open Decisions Deferred

- Folder discovery polling for newly created or deleted folders.
- Connection count limits.
- Gmail label support and cross-label deduplication.
- Unified all-mail view.
- Foxmail-style manual sync menu.

## Non-Goals

This stage does not try to become a full Foxmail clone. It focuses on making QQ automatic sync credible, event-driven, and easier to maintain.
