# Consistency Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace active IMAP IDLE watcher-driven automatic refresh with a provider-neutral consistency sync path.

**Architecture:** Keep raw IDLE support isolated for future use, but move the active automatic sync path to a backend `ConsistencySyncService`. The service owns startup, interval, foreground, account-save, and manual sync orchestration, reusing `AppApi::sync_account(...)` for mailbox correctness and `agentmail-mail-sync` for UI refreshes.

**Tech Stack:** Rust/Tauri v2, Tokio, React/Vite, TypeScript, Vitest, SQLite-backed `app-api`.

---

## File Structure

- Create `src-tauri/src/sync_events.rs`: neutral mail sync event payload and emitter used by both consistency sync and dormant IDLE watchers.
- Create `src-tauri/src/sync_service.rs`: backend consistency sync service, active-run registry, sync reason mapping, interval scheduler, foreground handler.
- Rename `src-tauri/src/watchers.rs` to `src-tauri/src/idle_watchers.rs`: dormant IDLE watcher module. Keep its tests and low-level behavior intact.
- Modify `src-tauri/src/main.rs`: manage `ConsistencySyncService`, route `sync_account` through it, add `run_foreground_sync`, remove active startup watcher launch.
- Modify `ui/src/lib/syncFlows.ts`: remove watcher startup from initial/manual sync helpers.
- Modify `ui/src/App.tsx`: remove watcher diagnostics listener and watcher calls, add foreground sync bridge.
- Modify `ui/src/api.ts`, `ui/src/data/demoBackend.ts`, `ui/src/api.test.ts`: remove frontend watcher binding and add foreground sync command.
- Modify `ui/src/App.test.ts`: update sync-flow expectations.
- Modify `docs/PROJECT_STATUS.md` and `docs/REAL_MAIL_ACCEPTANCE.md`: document consistency sync as the active strategy.

---

### Task 1: Extract Neutral Sync Events and Isolate IDLE Watchers

**Files:**
- Create: `src-tauri/src/sync_events.rs`
- Rename: `src-tauri/src/watchers.rs` -> `src-tauri/src/idle_watchers.rs`
- Modify: `src-tauri/src/idle_watchers.rs`
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Move the file**

Run:

```bash
git mv src-tauri/src/watchers.rs src-tauri/src/idle_watchers.rs
```

Expected: `git status --short` shows `R  src-tauri/src/watchers.rs -> src-tauri/src/idle_watchers.rs`.

- [ ] **Step 2: Create the neutral sync event module**

Add `src-tauri/src/sync_events.rs`:

```rust
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const MAIL_SYNC_EVENT: &str = "agentmail-mail-sync";

#[derive(Clone, Serialize)]
pub struct MailSyncEventPayload {
    pub account_id: String,
    pub folder_id: Option<String>,
    pub reason: &'static str,
    pub message: Option<String>,
}

pub fn emit_mail_sync_event(app: &AppHandle, payload: MailSyncEventPayload) {
    let _ = app.emit(MAIL_SYNC_EVENT, payload);
}
```

- [ ] **Step 3: Update the dormant IDLE module imports**

In `src-tauri/src/idle_watchers.rs`, remove `MAIL_SYNC_EVENT`, `MailSyncEventPayload`, and `emit_mail_sync_event` definitions. Add this import near the top:

```rust
use crate::sync_events::{emit_mail_sync_event, MailSyncEventPayload};
```

Keep `WATCH_DIAGNOSTIC_EVENT`, `WatchDiagnosticEventPayload`, `WatcherRegistry`, and all watcher tests in `idle_watchers.rs`.

- [ ] **Step 4: Update `main.rs` module imports**

At the top of `src-tauri/src/main.rs`, replace:

```rust
mod watchers;
```

with:

```rust
mod idle_watchers;
mod sync_events;
```

Replace the watcher import block:

```rust
use watchers::{
    emit_mail_sync_event, emit_watch_diagnostic, start_account_watchers_for_api,
    MailSyncEventPayload, WatchDiagnosticEventPayload, WatcherRegistry,
};
```

with:

```rust
use idle_watchers::{start_account_watchers_for_api, WatcherRegistry};
use sync_events::{emit_mail_sync_event, MailSyncEventPayload};
```

Leave `start_account_watchers(...)` wired for now. The active call sites are removed in Task 3.

- [ ] **Step 5: Verify event extraction compiles as far as the local Tauri environment allows**

Run:

```bash
cargo fmt --all --check
cargo test -p agentmail-app register_watcher
```

Expected: formatting passes. On a machine with Tauri Linux system libraries, the watcher tests pass. In this current Linux environment, `cargo test -p agentmail-app ...` may fail before tests run because Tauri GTK/WebKit system packages are missing; if that happens, record the exact missing package error and continue with the non-Tauri verification tasks later.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/main.rs src-tauri/src/idle_watchers.rs src-tauri/src/sync_events.rs
git commit -m "refactor: isolate idle watcher events"
```

---

### Task 2: Add the Consistency Sync Service

**Files:**
- Create: `src-tauri/src/sync_service.rs`
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Add the service module with testable pure helpers**

Create `src-tauri/src/sync_service.rs`:

```rust
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use app_api::{AppApi, SyncSummary};
use mail_core::MailAccount;
use tauri::AppHandle;

use crate::sync_events::{emit_mail_sync_event, MailSyncEventPayload};

pub const CONSISTENCY_SYNC_INTERVAL: Duration = Duration::from_secs(120);

#[derive(Clone, Default)]
pub struct SyncRunRegistry {
    active: Arc<Mutex<HashSet<String>>>,
}

pub struct SyncRunGuard {
    registry: SyncRunRegistry,
    account_id: String,
}

impl SyncRunRegistry {
    pub fn try_start(&self, account_id: &str) -> Option<SyncRunGuard> {
        let mut active = self.active.lock().ok()?;
        if !active.insert(account_id.to_string()) {
            return None;
        }
        Some(SyncRunGuard {
            registry: self.clone(),
            account_id: account_id.to_string(),
        })
    }

    pub fn is_active(&self, account_id: &str) -> bool {
        self.active
            .lock()
            .map(|active| active.contains(account_id))
            .unwrap_or(false)
    }
}

impl Drop for SyncRunGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.registry.active.lock() {
            active.remove(&self.account_id);
        }
    }
}

pub fn sync_enabled_account_ids(accounts: &[MailAccount]) -> Vec<String> {
    accounts
        .iter()
        .filter(|account| account.sync_enabled)
        .map(|account| account.id.clone())
        .collect()
}

pub fn sync_reason_from_request(reason: Option<String>) -> &'static str {
    match reason.as_deref() {
        Some("startup_sync") => "startup_sync",
        Some("account_saved_sync") => "account_saved_sync",
        Some("manual_sync") => "manual_sync",
        Some("foreground_sync") => "foreground_sync",
        Some("interval_sync") => "interval_sync",
        _ => "manual_sync",
    }
}

#[derive(Clone)]
pub struct ConsistencySyncService {
    api: Arc<AppApi>,
    app: AppHandle,
    registry: SyncRunRegistry,
}

impl ConsistencySyncService {
    pub fn new(api: Arc<AppApi>, app: AppHandle) -> Self {
        Self {
            api,
            app,
            registry: SyncRunRegistry::default(),
        }
    }

    pub async fn sync_account_once(
        &self,
        account_id: String,
        reason: &'static str,
    ) -> Result<SyncSummary, String> {
        let _guard = self
            .registry
            .try_start(&account_id)
            .ok_or_else(|| format!("sync already running: {account_id}"))?;
        let summary = self
            .api
            .sync_account(account_id)
            .await
            .map_err(|error| error.to_string())?;
        self.emit_summary(&summary, reason);
        Ok(summary)
    }

    pub async fn sync_account_if_idle(
        &self,
        account_id: String,
        reason: &'static str,
    ) -> Option<Result<SyncSummary, String>> {
        let guard = self.registry.try_start(&account_id)?;
        let result = self
            .api
            .sync_account(account_id)
            .await
            .map_err(|error| error.to_string());
        drop(guard);
        if let Ok(summary) = &result {
            self.emit_summary(summary, reason);
        }
        Some(result)
    }

    pub async fn sync_enabled_accounts(&self, reason: &'static str) {
        let Ok(accounts) = self.api.list_accounts() else {
            return;
        };
        for account_id in sync_enabled_account_ids(&accounts) {
            let _ = self.sync_account_if_idle(account_id, reason).await;
        }
    }

    pub async fn handle_foreground_resume(&self, selected_account_id: Option<String>) {
        if let Some(account_id) = selected_account_id {
            if self
                .api
                .is_account_sync_enabled(&account_id)
                .unwrap_or(false)
            {
                let _ = self
                    .sync_account_if_idle(account_id, "foreground_sync")
                    .await;
            }
            return;
        }
        self.sync_enabled_accounts("foreground_sync").await;
    }

    pub fn start_interval_sync(&self) {
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(CONSISTENCY_SYNC_INTERVAL).await;
                service.sync_enabled_accounts("interval_sync").await;
            }
        });
    }

    fn emit_summary(&self, summary: &SyncSummary, reason: &'static str) {
        emit_mail_sync_event(
            &self.app,
            MailSyncEventPayload {
                account_id: summary.account_id.clone(),
                folder_id: None,
                reason,
                message: Some(format!(
                    "{} folders / {} messages",
                    summary.folders, summary.messages
                )),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, sync_enabled: bool) -> MailAccount {
        MailAccount {
            id: id.to_string(),
            display_name: id.to_string(),
            email: format!("{id}@example.com"),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled,
            created_at: "2026-04-29T00:00:00Z".to_string(),
            updated_at: "2026-04-29T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn sync_run_registry_prevents_overlap_for_same_account() {
        let registry = SyncRunRegistry::default();
        let first = registry.try_start("acct-1");

        assert!(first.is_some());
        assert!(registry.try_start("acct-1").is_none());
        assert!(registry.try_start("acct-2").is_some());
    }

    #[test]
    fn sync_run_guard_releases_account_on_drop() {
        let registry = SyncRunRegistry::default();

        {
            let _guard = registry.try_start("acct-1").expect("first sync starts");
            assert!(registry.is_active("acct-1"));
        }

        assert!(!registry.is_active("acct-1"));
        assert!(registry.try_start("acct-1").is_some());
    }

    #[test]
    fn sync_enabled_account_ids_filters_disabled_accounts() {
        let accounts = vec![account("enabled", true), account("disabled", false)];

        assert_eq!(sync_enabled_account_ids(&accounts), vec!["enabled".to_string()]);
    }

    #[test]
    fn sync_reason_from_request_allows_known_reasons_only() {
        assert_eq!(
            sync_reason_from_request(Some("account_saved_sync".to_string())),
            "account_saved_sync"
        );
        assert_eq!(
            sync_reason_from_request(Some("unexpected".to_string())),
            "manual_sync"
        );
        assert_eq!(sync_reason_from_request(None), "manual_sync");
    }
}
```

- [ ] **Step 2: Register the module in `main.rs`**

At the top of `src-tauri/src/main.rs`, add:

```rust
mod sync_service;
```

Add this import:

```rust
use sync_service::{sync_reason_from_request, ConsistencySyncService};
```

- [ ] **Step 3: Verify pure service tests**

Run:

```bash
cargo fmt --all --check
cargo test -p agentmail-app sync_service
```

Expected: formatting passes. On a machine with Tauri system libraries, the `sync_service` unit tests pass. If this environment fails before tests run due to missing Tauri GTK/WebKit libraries, keep the code and record the blocker in the final verification notes.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/main.rs src-tauri/src/sync_service.rs
git commit -m "feat: add consistency sync service"
```

---

### Task 3: Wire Tauri Startup, Manual, Foreground, and Interval Sync Through the Service

**Files:**
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Update the `sync_account` command signature**

Replace the current command:

```rust
#[tauri::command]
async fn sync_account(
    state: State<'_, ApiState>,
    account_id: String,
) -> Result<SyncSummary, String> {
    state.api.sync_account(account_id).await.map_err(to_error)
}
```

with:

```rust
#[tauri::command]
async fn sync_account(
    sync_service: State<'_, ConsistencySyncService>,
    account_id: String,
    reason: Option<String>,
) -> Result<SyncSummary, String> {
    sync_service
        .sync_account_once(account_id, sync_reason_from_request(reason))
        .await
        .map_err(to_error)
}
```

- [ ] **Step 2: Add the foreground sync command**

Add this command after `sync_account(...)`:

```rust
#[tauri::command]
async fn run_foreground_sync(
    sync_service: State<'_, ConsistencySyncService>,
    selected_account_id: Option<String>,
) -> Result<(), String> {
    sync_service
        .handle_foreground_resume(selected_account_id)
        .await;
    Ok(())
}
```

- [ ] **Step 3: Replace setup watcher orchestration with consistency sync**

In `setup(...)`, keep `let api = Arc::new(api);` and remove:

```rust
let watcher_registry = WatcherRegistry::default();
```

Then replace:

```rust
app.manage(watcher_registry.clone());
let app_handle = app.handle().clone();
tauri::async_runtime::spawn(async move {
    if let Ok(accounts) = api.list_accounts() {
        for account in accounts.into_iter().filter(|account| account.sync_enabled) {
            let account_id = account.id;
            if let Ok(summary) = api.sync_account(account_id.clone()).await {
                emit_mail_sync_event(
                    &app_handle,
                    MailSyncEventPayload {
                        account_id: summary.account_id,
                        folder_id: None,
                        reason: "startup_sync",
                        message: Some(format!(
                            "{} folders / {} messages",
                            summary.folders, summary.messages
                        )),
                    },
                );
                if let Err(error) = start_account_watchers_for_api(
                    Arc::clone(&api),
                    watcher_registry.clone(),
                    app_handle.clone(),
                    account_id.clone(),
                ) {
                    emit_watch_diagnostic(
                        &app_handle,
                        WatchDiagnosticEventPayload {
                            account_id,
                            folder_id: None,
                            stage: "watch_start_failed",
                            message: Some(error),
                        },
                    );
                }
            }
        }
    }
});
```

with:

```rust
let sync_service = ConsistencySyncService::new(Arc::clone(&api), app.handle().clone());
app.manage(sync_service.clone());
let startup_sync = sync_service.clone();
tauri::async_runtime::spawn(async move {
    startup_sync.sync_enabled_accounts("startup_sync").await;
});
sync_service.start_interval_sync();
```

- [ ] **Step 4: Keep the dormant IDLE command managed but inactive**

After `app.manage(sync_service.clone());`, add:

```rust
app.manage(WatcherRegistry::default());
```

Keep `start_account_watchers` in the invoke handler for future internal use, but do not call it from setup or UI.

- [ ] **Step 5: Add the new command to the invoke handler**

In `tauri::generate_handler![...]`, add `run_foreground_sync` immediately after `sync_account`:

```rust
sync_account,
run_foreground_sync,
start_account_watchers,
```

- [ ] **Step 6: Remove unused imports**

After the setup rewrite, `main.rs` should no longer import `emit_mail_sync_event` or `MailSyncEventPayload`. `start_account_watchers_for_api` remains only for the dormant command. The remaining imports should look like:

```rust
use idle_watchers::{start_account_watchers_for_api, WatcherRegistry};
use sync_service::{sync_reason_from_request, ConsistencySyncService};
```

Do not import `emit_watch_diagnostic` or `WatchDiagnosticEventPayload` in `main.rs`.

- [ ] **Step 7: Verify the Tauri wiring**

Run:

```bash
cargo fmt --all --check
cargo test -p agentmail-app sync_service
```

Expected: formatting passes. App-crate tests pass where Tauri system dependencies are installed; otherwise record the same missing-library blocker.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/main.rs src-tauri/src/sync_service.rs
git commit -m "feat: route automatic sync through consistency service"
```

---

### Task 4: Remove Watcher Startup From Frontend Sync Flows

**Files:**
- Modify: `ui/src/lib/syncFlows.ts`
- Modify: `ui/src/App.test.ts`
- Modify: `ui/src/App.tsx`

- [ ] **Step 1: Update failing tests for initial sync**

In `ui/src/App.test.ts`, change the first `runInitialAccountSync` test name to:

```ts
it("syncs a saved account, refreshes observable state, and returns a useful status", async () => {
```

Remove the local `startAccountWatchers` mock from that test, remove `startAccountWatchers` from the helper call, and replace:

```ts
expect(startAccountWatchers).toHaveBeenCalledWith("acct-1");
```

with:

```ts
expect(refreshFolders).toHaveBeenCalledWith("acct-1");
```

Delete the entire test named:

```ts
it("keeps the saved account path alive when watcher startup fails after a successful sync", async () => {
```

In the disabled-sync test, remove the `startAccountWatchers` mock, remove it from the helper call, and delete:

```ts
expect(startAccountWatchers).not.toHaveBeenCalled();
```

- [ ] **Step 2: Update failing tests for manual sync**

In `ui/src/App.test.ts`, change:

```ts
it("syncs the account, restarts watchers, and refreshes visible state", async () => {
```

to:

```ts
it("syncs the account and refreshes visible state", async () => {
```

Remove the `startAccountWatchers` mock, remove it from the `runManualAccountSync(...)` call, and delete:

```ts
expect(startAccountWatchers).toHaveBeenCalledWith("acct-1");
```

Delete the entire test named:

```ts
it("keeps manual sync successful while reporting watcher restart failures", async () => {
```

- [ ] **Step 3: Run the focused test to verify it fails**

Run:

```bash
pnpm test -- ui/src/App.test.ts
```

Expected: FAIL because `runInitialAccountSync` and `runManualAccountSync` still accept and call `startAccountWatchers`.

- [ ] **Step 4: Remove watcher fields and warning helper from `syncFlows.ts`**

In `ui/src/lib/syncFlows.ts`, delete:

```ts
function appendWatcherWarning(status: string, watcherError: unknown) {
  return watcherError ? `${status} / watcher start failed: ${String(watcherError)}` : status;
}
```

Remove this field from `InitialAccountSyncRequest`:

```ts
startAccountWatchers?: (accountId: string) => Promise<unknown>;
```

Remove `startAccountWatchers` and `watcherError` from `runInitialAccountSync(...)`. The successful return should become:

```ts
return `account saved and initial sync complete: ${email} / ${summary.folders} folders / ${summary.messages} messages`;
```

Remove this field from `ManualAccountSyncRequest`:

```ts
startAccountWatchers?: (accountId: string) => Promise<unknown>;
```

Remove `startAccountWatchers` and `watcherError` from `runManualAccountSync(...)`. The final return should become:

```ts
return `sync complete: ${summary.folders} folders / ${summary.messages} messages`;
```

- [ ] **Step 5: Remove frontend watcher calls in `App.tsx`**

In `handleSync`, remove:

```ts
startAccountWatchers: api.startAccountWatchers,
```

and change:

```ts
syncAccount: api.syncAccount,
```

to:

```ts
syncAccount: (accountId) => api.syncAccount(accountId, "manual_sync"),
```

In `handleAccountConfigSaved`, remove:

```ts
startAccountWatchers: api.startAccountWatchers,
```

and change:

```ts
syncAccount: api.syncAccount,
```

to:

```ts
syncAccount: (accountId) => api.syncAccount(accountId, "account_saved_sync"),
```

- [ ] **Step 6: Remove the active watcher diagnostic listener from `App.tsx`**

Delete the `WATCH_DIAGNOSTIC_EVENT` export, the `WatchDiagnosticEventPayload` type, and the `useEffect` block that listens to `WATCH_DIAGNOSTIC_EVENT`.

Keep `MAIL_SYNC_EVENT` and the `refreshAfterMailSyncEvent(...)` listener.

- [ ] **Step 7: Verify frontend sync-flow tests**

Run:

```bash
pnpm test -- ui/src/App.test.ts
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add ui/src/lib/syncFlows.ts ui/src/App.test.ts ui/src/App.tsx
git commit -m "refactor: remove watcher startup from sync flows"
```

---

### Task 5: Add Frontend API Support for Foreground Consistency Sync

**Files:**
- Modify: `ui/src/api.ts`
- Modify: `ui/src/data/demoBackend.ts`
- Modify: `ui/src/api.test.ts`
- Modify: `ui/src/App.tsx`

- [ ] **Step 1: Update the API bindings**

In `ui/src/api.ts`, remove this `CommandMap` field:

```ts
start_account_watchers: null;
```

Add this field after `sync_account`:

```ts
run_foreground_sync: null;
```

Replace:

```ts
syncAccount: (accountId: string) => call("sync_account", { accountId }),
startAccountWatchers: (accountId: string) => call("start_account_watchers", { accountId }),
```

with:

```ts
syncAccount: (accountId: string, reason = "manual_sync") => call("sync_account", { accountId, reason }),
runForegroundSync: (selectedAccountId: string | null) =>
  call("run_foreground_sync", { selectedAccountId }),
```

- [ ] **Step 2: Update the browser demo backend**

In `ui/src/data/demoBackend.ts`, replace:

```ts
case "start_account_watchers":
  return null;
```

with:

```ts
case "run_foreground_sync":
  return null;
```

The existing `sync_account` demo case can ignore the new `reason` argument.

- [ ] **Step 3: Update API tests**

In `ui/src/api.test.ts`, replace:

```ts
it("supports starting account watchers in the browser demo", async () => {
  await expect(api.startAccountWatchers("demo-account")).resolves.toBeNull();
});
```

with:

```ts
it("supports foreground sync in the browser demo", async () => {
  await expect(api.runForegroundSync("demo-account")).resolves.toBeNull();
  await expect(api.runForegroundSync(null)).resolves.toBeNull();
});
```

- [ ] **Step 4: Add foreground sync bridge to `App.tsx`**

After the refs near `queryRef`, add:

```ts
const lastForegroundSyncAtRef = useRef(0);
```

Add this callback after the mail sync event listener:

```ts
const requestForegroundSync = useCallback(() => {
  if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
  const now = Date.now();
  if (now - lastForegroundSyncAtRef.current < 30_000) return;
  lastForegroundSyncAtRef.current = now;
  const accountId = selectedAccountIdRef.current;
  void api
    .runForegroundSync(accountId)
    .then(() => appendActivityLog(`foreground sync requested: ${accountId ?? "all accounts"}`))
    .catch((error) => appendActivityLog(`foreground sync request failed: ${String(error)}`));
}, [appendActivityLog]);
```

Add this effect after the callback:

```ts
useEffect(() => {
  if (typeof window === "undefined") return undefined;
  const handleVisibilityChange = () => {
    if (document.visibilityState === "visible") requestForegroundSync();
  };
  window.addEventListener("focus", requestForegroundSync);
  document.addEventListener("visibilitychange", handleVisibilityChange);
  return () => {
    window.removeEventListener("focus", requestForegroundSync);
    document.removeEventListener("visibilitychange", handleVisibilityChange);
  };
}, [requestForegroundSync]);
```

This keeps the foreground trigger in the UI because the backend needs the selected account id, but the actual mailbox sync runs through the backend service.

- [ ] **Step 5: Verify TypeScript tests**

Run:

```bash
pnpm test -- ui/src/api.test.ts ui/src/App.test.ts
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add ui/src/api.ts ui/src/data/demoBackend.ts ui/src/api.test.ts ui/src/App.tsx
git commit -m "feat: request foreground consistency sync"
```

---

### Task 6: Update Current Docs and Acceptance Criteria

**Files:**
- Modify: `docs/PROJECT_STATUS.md`
- Modify: `docs/REAL_MAIL_ACCEPTANCE.md`

- [ ] **Step 1: Update `docs/PROJECT_STATUS.md`**

Replace:

```markdown
- Direct-actions cleanup and QQ dynamic IMAP IDLE sync are implemented on `main`; this status includes direct-send result hardening, browser-demo parity fixes, and QQ folder watcher work from the 2026-04-28 series.
```

with:

```markdown
- Direct-actions cleanup and consistency-first automatic sync are implemented on `main`; this status includes direct-send result hardening, browser-demo parity fixes, and the 2026-04-29 shift away from active QQ IDLE watcher sync.
```

Replace:

```markdown
- QQ Mail automatic refresh uses IMAP IDLE watchers for the account's stored selectable folders.
- Manual sync is still available and reconciles watchers after an explicit account sync.
- Folder create/delete discovery is not automatic in this stage; startup, account save, or manual sync refreshes the folder list.
```

with:

```markdown
- Automatic refresh is consistency-sync driven for all enabled accounts; startup, foreground resume, a 120-second running-app interval, account save, and manual sync all use account-level sync.
- IMAP IDLE watcher code remains isolated for future providers, but it is not part of the active sync path in this stage.
- Folder create/delete discovery is not realtime in this stage; startup, account save, interval, foreground, or manual sync refreshes the folder list.
```

Replace the recent verification heading:

```markdown
For the QQ dynamic IMAP IDLE sync work, the following checks were run on 2026-04-28:
```

with:

```markdown
For the consistency-sync architecture work, the following checks should be run on 2026-04-29:
```

Remove the `rg -n "AUTO_SYNC_INTERVAL_MS|runAutomaticAccountSync|setInterval\\(runAutoSync|auto sync complete" ui/src` verification bullet and replace it with:

```markdown
- `rg -n "startAccountWatchers|watcher start failed|WATCH_DIAGNOSTIC_EVENT" ui/src` returned no matches.
```

- [ ] **Step 2: Update `docs/REAL_MAIL_ACCEPTANCE.md`**

Replace the Sync Acceptance bullets:

```markdown
- QQ automatic sync is IMAP IDLE-driven for all selectable folders returned by the account's IMAP folder list.
- Background automatic sync must not use a 30-second polling interval.
- Manual sync remains available from the topbar, shows a waiting ring while active, and refreshes folders/messages/sync state/audits after completion.
```

with:

```markdown
- Automatic sync is consistency-driven for every enabled account, not dependent on QQ IMAP IDLE push behavior.
- While the app is running, enabled accounts sync at startup, after account save, on foreground resume, on a 120-second interval, and through manual sync.
- Manual sync remains available from the topbar, shows a waiting ring while active, and refreshes folders/messages/sync state/audits after completion.
```

Add this known limit:

```markdown
- IMAP IDLE watcher code is retained for future provider support, but it is dormant in the active sync path.
```

- [ ] **Step 3: Verify docs do not claim active QQ IDLE sync**

Run:

```bash
rg -n "QQ automatic sync is IMAP IDLE-driven|QQ Mail automatic refresh uses IMAP IDLE watchers|watcher start failed" docs/PROJECT_STATUS.md docs/REAL_MAIL_ACCEPTANCE.md ui/src
```

Expected: no matches.

- [ ] **Step 4: Commit**

```bash
git add docs/PROJECT_STATUS.md docs/REAL_MAIL_ACCEPTANCE.md
git commit -m "docs: document consistency sync as active strategy"
```

---

### Task 7: Full Verification

**Files:**
- No planned source edits unless verification exposes an issue.

- [ ] **Step 1: Format check**

Run:

```bash
cargo fmt --all --check
```

Expected: PASS.

- [ ] **Step 2: Rust workspace tests**

Run:

```bash
pnpm rust:test
```

Expected: PASS for `mail-core`, `mail-store`, `mail-protocol`, `ai-remote`, and `app-api`.

- [ ] **Step 3: Tauri app tests when environment supports them**

Run:

```bash
cargo test -p agentmail-app sync_service
```

Expected: PASS on a machine with required Tauri Linux system libraries. In the current Linux environment this may be blocked by missing packages such as `atk`, `gio-2.0`, `glib-2.0`, `javascriptcoregtk-4.1`, `libsoup-3.0`, `gdk-3.0`, `gdk-pixbuf-2.0`, `pango`, or `cairo`; if blocked, record the exact message.

- [ ] **Step 4: Frontend tests**

Run:

```bash
pnpm test
```

Expected: PASS.

- [ ] **Step 5: Frontend production build**

Run:

```bash
pnpm build
```

Expected: PASS.

- [ ] **Step 6: Non-Tauri Rust check**

Run:

```bash
pnpm rust:check
```

Expected: PASS.

- [ ] **Step 7: Search for removed active watcher UI paths**

Run:

```bash
rg -n "startAccountWatchers|watcher start failed|WATCH_DIAGNOSTIC_EVENT" ui/src
```

Expected: no matches.

- [ ] **Step 8: Search for active startup watcher launch**

Run:

```bash
rg -n "start_account_watchers_for_api\\(|emit_watch_diagnostic\\(" src-tauri/src/main.rs
```

Expected: no matches. It is acceptable for `start_account_watchers_for_api` and `emit_watch_diagnostic` to remain inside `src-tauri/src/idle_watchers.rs`.

- [ ] **Step 9: Final status**

Run:

```bash
git status --short --branch
```

Expected: only intentionally untracked local files remain, currently `log.txt` and `scripts/`. No implementation files should be unstaged.

---

## Execution Notes

- Do not add `log.txt` to any commit.
- Do not commit credentials or edited probe scripts with real mailbox secrets.
- Keep IMAP IDLE protocol support compile-valid, but do not let startup, account-save, foreground, interval, or manual sync start IDLE watchers.
- The active product path after this plan is consistency sync, not provider-specific realtime sync.
