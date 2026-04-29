# Consistency Sync Design

## Goal

Move AgentMail's automatic mail refresh back to a consistency-first sync model.
IMAP IDLE should remain as a reusable lower-level capability for future providers,
but it should not be part of the active business sync path in this stage.

The immediate product goal is simple: while the app is running, enabled accounts
should converge with the remote mailbox reliably, even when a provider advertises
IMAP IDLE but does not deliver IDLE change events.

## Current Context

The current implementation has two separate ideas coupled together:

- `mail-protocol` exposes `watch_folder_until_change(...)`, which performs the
  raw IMAP IDLE flow: login, `CAPABILITY`, `SELECT`, enter IDLE, then return
  `Changed` or `Timeout`.
- `src-tauri/src/watchers.rs` starts a watcher for each stored folder and calls
  `sync_folder(...)` when IDLE reports `Changed`.
- Startup sync, account-save sync, and manual sync all try to start or reconcile
  watchers after a successful account sync.
- The previous frontend periodic automatic sync path has already been removed.

Live QQ Mail testing showed that this is not a reliable business foundation:
QQ accepted IDLE and delivered the message to `INBOX`, but it did not push an
IDLE `EXISTS` update during the active wait window. The app therefore needs a
sync architecture that treats IDLE as an optional realtime accelerator, not as
the only automatic update mechanism.

## Scope

### In Scope

- Use one consistency sync strategy for all providers in this stage.
- Keep existing `sync_account(account_id)` as the main consistency-sync entry.
- Add a backend-owned automatic sync scheduler in Tauri.
- Trigger consistency sync on startup, account save, manual sync, foreground
  resume, and a conservative running-app interval.
- Remove active watcher startup from startup, account save, and manual sync
  flows.
- Keep the low-level IDLE code available but dormant.
- Keep `sync_folder(account_id, folder_id)` for future realtime or targeted sync.
- Keep the `agentmail-mail-sync` event bridge so the UI refreshes after backend
  sync completion.
- Update tests and documentation so the product no longer claims QQ automatic
  refresh is IDLE-driven.

### Out Of Scope

- Provider-specific policy tables for QQ, Gmail, Outlook, or custom IMAP.
- Tray/background daemon behavior after the app window is closed.
- OS notifications for newly arrived mail.
- Push-style provider integrations such as Gmail Pub/Sub or Microsoft Graph.
- User-facing sync interval settings.
- Attachment download or attachment cache behavior.
- Large unrelated refactors of message storage, actions, AI, or theme code.

## Business Rules

1. `sync_enabled=false` means the account is skipped by all automatic sync
   triggers.
2. Startup sync runs for every `sync_enabled=true` account and emits
   `agentmail-mail-sync` after each successful account sync.
3. Saving an account with sync enabled runs one initial consistency sync for that
   account, but no longer starts IDLE watchers.
4. Manual sync runs an immediate account sync for the selected account and keeps
   the existing visible waiting state.
5. Foreground resume runs a sync for the selected enabled account. If there is no
   selected account, it syncs all enabled accounts.
6. Interval sync runs only while the app process is active. The first interval
   should be 120 seconds.
7. Per-account overlap must be prevented. If one account is already syncing, a
   second automatic trigger for that same account is skipped.
8. Existing `AppApi::sync_account(...)` lock, backoff, full-folder snapshot, and
   reconciliation behavior remain authoritative.
9. IDLE code remains compile-tested and isolated, but no active product path
   starts IDLE watchers in this stage.

## Architecture

### Protocol Layer

`mail-protocol` keeps the raw mail operations:

- `fetch_folders(settings, account)`
- `fetch_messages(settings, account, folder, request)`
- `apply_action(settings, account, action)`
- `watch_folder_until_change(settings, account, folder)`

No provider policy should be added here. The protocol layer should remain a thin
IMAP/SMTP adapter.

### App API Layer

`app-api` remains the authoritative domain API:

- `sync_account(account_id)` is the consistency sync entry point.
- `sync_folder(account_id, folder_id)` remains available for future targeted sync.
- `watch_folder_until_change(account_id, folder_id)` can remain available for the
  dormant IDLE module.

This layer already has the important correctness behavior: sync locks, backoff,
folder ownership checks, full-folder snapshots, and stale UID reconciliation. The
new scheduler should reuse it instead of duplicating mailbox logic.

### Tauri Consistency Sync Service

Add a focused backend module, suggested path:

```text
src-tauri/src/sync_service.rs
```

The service owns automatic sync orchestration:

```text
ConsistencySyncService
  api: Arc<AppApi>
  app: AppHandle
  active_accounts: HashSet<account_id>

sync_account_once(account_id, reason)
sync_enabled_accounts(reason)
start_interval_sync()
handle_foreground_resume(selected_account_id)
emit_sync_event(summary, reason)
```

The service should not own mail protocol details. It calls `AppApi::sync_account`
and translates the outcome into diagnostics and `agentmail-mail-sync` events.

Recommended reasons:

- `startup_sync`
- `account_saved_sync`
- `manual_sync`
- `foreground_sync`
- `interval_sync`

### Dormant IDLE Module

Keep the existing watcher code, but rename or isolate it so its role is clear.
Suggested naming:

```text
src-tauri/src/idle_watchers.rs
```

The module may still expose `start_account_watchers_for_api(...)`, but it should
not be called by startup, account save, or manual sync in this stage. Keeping it
compile-tested preserves future support for providers whose IDLE behavior is
verified to be reliable.

If the Tauri command `start_account_watchers` remains exported for internal
testing or future use, it should not be called by the UI by default.

## Data Flow

### Startup

```text
Tauri setup
-> initialize AppApi
-> initialize ConsistencySyncService
-> list sync-enabled accounts
-> sync_account_once(account_id, "startup_sync")
-> emit agentmail-mail-sync
```

### Account Save

```text
User saves account
-> save_account_config
-> if sync_enabled: sync_account_once(account_id, "account_saved_sync")
-> refresh folders/messages/sync state/audits in UI
```

### Manual Sync

```text
User clicks sync
-> isManualSyncing = true
-> sync_account_once(account_id, "manual_sync")
-> refresh visible UI state
-> isManualSyncing = false
```

### Foreground Resume

```text
Window/app becomes active
-> selected account exists and sync_enabled
-> sync_account_once(account_id, "foreground_sync")
-> emit agentmail-mail-sync
```

### Interval Sync

```text
Every 120 seconds while app process is running
-> sync_enabled_accounts("interval_sync")
-> for each enabled account, skip if already active or in backoff
-> emit agentmail-mail-sync for successful syncs
```

## Frontend Behavior

The frontend should stop treating watchers as part of the normal sync flow.

Required changes:

- Remove `startAccountWatchers` from `runInitialAccountSync`.
- Remove `startAccountWatchers` from `runManualAccountSync`.
- Remove `appendWatcherWarning(...)` and watcher failure status text.
- Keep `refreshAfterMailSyncEvent(...)` as the UI refresh bridge.
- Keep manual sync pending state and disable duplicate manual sync clicks.
- Add a foreground-resume bridge if the backend needs the selected account id for
  targeted foreground sync.

The UI should use provider-neutral status language:

- `account saved and initial sync complete`
- `sync complete`
- `auto sync complete`
- `sync skipped`
- `sync failed`

It should not show `IDLE`, `watcher`, or `push` language for the active sync path.

## Error Handling

- Sync disabled: skip automatic sync without surfacing an error.
- Account already syncing: skip the automatic trigger; manual sync can
  show the existing sync-in-progress error if that is the current API behavior.
- Backoff active: respect the existing `SyncBackoff` result and do not clear it
  from the scheduler.
- Network or protocol failure: rely on `sync_account` to store backoff state and
  error message.
- One account failure: do not stop syncing other enabled accounts.
- UI refresh failure after a backend sync event: log/report the refresh failure
  locally, but do not mark the backend sync itself as failed.

## Testing Plan

### Rust

- `ConsistencySyncService` skips accounts with `sync_enabled=false`.
- `ConsistencySyncService` prevents overlapping syncs for the same account.
- Startup sync emits `agentmail-mail-sync` for each successful enabled account.
- Interval sync uses the configured interval and calls account sync, not IDLE
  watcher startup.
- Foreground sync targets the selected account when one is available.
- Existing `sync_account` and `sync_folder` tests continue to pass.
- Existing IDLE/watch tests remain compile-valid but do not imply active business
  use.

### TypeScript

- Initial account sync no longer calls `startAccountWatchers`.
- Manual sync no longer calls `startAccountWatchers`.
- Watcher warning text is removed from sync status results.
- `refreshAfterMailSyncEvent` still refreshes folders, messages, sync state, and
  audits for the selected account.
- Manual sync pending state still disables the sync button and stops on success or
  failure.

### Documentation Checks

- `docs/PROJECT_STATUS.md` no longer says QQ automatic refresh is IDLE-driven
  after implementation.
- `docs/REAL_MAIL_ACCEPTANCE.md` describes consistency sync and does not promise
  realtime QQ IDLE updates.
- Historical IDLE design docs remain as traceability, but current status docs make
  the new architecture clear.

## Acceptance Criteria

- No active startup, account-save, or manual-sync path starts IDLE watchers.
- Automatic sync works through the backend consistency sync service.
- Enabled accounts sync at startup and on the 120-second running-app interval.
- Manual sync still works and refreshes the visible UI.
- Foreground resume triggers a consistency sync for the selected enabled account.
- `agentmail-mail-sync` remains the single UI refresh event for backend sync
  completion.
- IDLE protocol code remains present and isolated for future provider support.
- Tests and current status docs reflect consistency sync as the active strategy.
