# QQ Dynamic IMAP IDLE Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make QQ Mail automatic sync event-driven with dynamic IMAP IDLE watchers for every selectable folder, while preserving manual sync and adding a manual-sync waiting ring.

**Architecture:** Add a backend `sync_folder(account_id, folder_id)` path first so IDLE events update only the changed folder. Move watcher orchestration from `src-tauri/src/main.rs` into `src-tauri/src/watchers.rs`, then remove the frontend 30-second automatic sync interval and make manual sync visibly pending.

**Tech Stack:** Rust workspace (`app-api`, `mail-protocol`, `mail-store`, `mail-core`), Tauri v2 Rust shell, React/Vite TypeScript UI, Vitest, Cargo, pnpm.

---

## File Structure

- Modify `crates/app-api/src/lib.rs`: add `sync_folder`, extract reusable single-folder sync helper from `sync_account_inner`, and add focused tests.
- Create `src-tauri/src/watchers.rs`: own `MAIL_SYNC_EVENT`, `WatcherRegistry`, `MailSyncEventPayload`, watcher registration, folder watcher loop, and Tauri event emission.
- Modify `src-tauri/src/main.rs`: import the watcher module, remove inline watcher functions/types, and keep commands/setup wired through the new module.
- Modify `ui/src/lib/syncFlows.ts`: remove `runAutomaticAccountSync`; keep manual, initial, direct send, and sync-event helpers.
- Create `ui/src/lib/syncUi.ts`: pure manual-sync button state helper.
- Modify `ui/src/App.tsx`: remove 30-second automatic sync effect, add `isManualSyncing`, apply sync button waiting state, and keep manual sync behavior.
- Modify `ui/src/App.test.ts`: remove automatic sync tests, add sync button state tests, and update imports.
- Modify `ui/src/styles/app.css`: add manual sync icon spin/waiting ring styling.
- Modify `docs/REAL_MAIL_ACCEPTANCE.md`: document QQ IDLE-only acceptance and manual sync waiting ring.
- Modify `docs/PROJECT_STATUS.md`: update current sync behavior summary after implementation.

## Task 1: Add Backend Folder-Level Sync

**Files:**
- Modify: `crates/app-api/src/lib.rs`

- [ ] **Step 1: Add failing tests for folder-scoped sync**

In `crates/app-api/src/lib.rs`, inside `mod tests`, add this helper near `SnapshotSyncProtocol`:

```rust
struct FolderScopedSyncProtocol {
    requests: Arc<Mutex<Vec<String>>>,
}

impl FolderScopedSyncProtocol {
    fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl MailProtocol for FolderScopedSyncProtocol {
    async fn test_connection(
        &self,
        _settings: &ConnectionSettings,
    ) -> ProtocolResult<ConnectionTestResult> {
        Ok(ConnectionTestResult {
            imap_ok: true,
            smtp_ok: true,
            message: "ok".to_string(),
        })
    }

    async fn fetch_folders(
        &self,
        _settings: &ConnectionSettings,
        account: &MailAccount,
    ) -> ProtocolResult<Vec<MailFolder>> {
        Ok(vec![
            MailFolder {
                id: format!("{}:inbox", account.id),
                account_id: account.id.clone(),
                name: "INBOX".to_string(),
                path: "INBOX".to_string(),
                role: FolderRole::Inbox,
                unread_count: 0,
                total_count: 0,
            },
            MailFolder {
                id: format!("{}:archive", account.id),
                account_id: account.id.clone(),
                name: "Archive".to_string(),
                path: "Archive".to_string(),
                role: FolderRole::Archive,
                unread_count: 0,
                total_count: 0,
            },
        ])
    }

    async fn fetch_messages(
        &self,
        _settings: &ConnectionSettings,
        account: &MailAccount,
        folder: &MailFolder,
        _request: &MessageFetchRequest,
    ) -> ProtocolResult<Vec<MailMessage>> {
        self.requests.lock().push(folder.id.clone());
        Ok(vec![MailMessage {
            id: format!("{}:{}:uid-1", account.id, folder.id),
            account_id: account.id.clone(),
            folder_id: folder.id.clone(),
            uid: Some("1".to_string()),
            message_id_header: Some(format!("<{}-uid-1@example.com>", folder.id)),
            subject: format!("{} update", folder.name),
            sender: "ops@example.com".to_string(),
            recipients: vec![account.email.clone()],
            cc: Vec::new(),
            received_at: now_rfc3339(),
            body_preview: "body".to_string(),
            body: Some("body".to_string()),
            attachments: Vec::new(),
            flags: mail_core::MessageFlags {
                is_read: false,
                is_starred: false,
                is_answered: false,
                is_forwarded: false,
            },
            size_bytes: Some(128),
            deleted_at: None,
        }])
    }

    async fn send_message(
        &self,
        _settings: &ConnectionSettings,
        _draft: &SendMessageDraft,
    ) -> ProtocolResult<String> {
        Ok(new_id())
    }

    async fn apply_action(
        &self,
        _settings: &ConnectionSettings,
        _account: &MailAccount,
        _action: &RemoteMailAction,
    ) -> ProtocolResult<()> {
        Ok(())
    }
}
```

Add these tests near the existing sync tests:

```rust
#[tokio::test]
async fn sync_folder_fetches_only_the_target_folder() {
    let protocol = FolderScopedSyncProtocol::new();
    let requests = Arc::clone(&protocol.requests);
    let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
    let account = add_sample_account(&api).await;
    api.sync_account(account.id.clone()).await.unwrap();
    requests.lock().clear();

    let archive = api
        .store
        .find_folder_by_role(&account.id, FolderRole::Archive)
        .unwrap()
        .unwrap();
    let summary = api
        .sync_folder(account.id.clone(), archive.id.clone())
        .await
        .unwrap();

    assert_eq!(summary.account_id, account.id);
    assert_eq!(summary.folders, 1);
    assert_eq!(requests.lock().clone(), vec![archive.id.clone()]);
    let state = api
        .store
        .get_sync_state(&account.id, Some(&archive.id))
        .unwrap()
        .unwrap();
    assert_eq!(state.state, SyncStateKind::Idle);
    assert_eq!(api.store.get_folder(&archive.id).unwrap().total_count, 1);
}

#[tokio::test]
async fn sync_folder_rejects_folder_from_another_account() {
    let api = AppApi::new(
        MailStore::memory().unwrap(),
        Arc::new(FolderScopedSyncProtocol::new()),
    );
    let first = add_sample_account(&api).await;
    let second = api
        .save_account_config(SaveAccountConfigRequest {
            id: None,
            display_name: "Other".to_string(),
            email: "other@example.com".to_string(),
            password: "secret".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled: true,
        })
        .await
        .unwrap();
    api.sync_account(first.id.clone()).await.unwrap();
    let inbox = api
        .store
        .find_folder_by_role(&first.id, FolderRole::Inbox)
        .unwrap()
        .unwrap();

    let error = api
        .sync_folder(second.id, inbox.id)
        .await
        .unwrap_err();

    assert!(matches!(error, ApiError::InvalidRequest(_)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p app-api sync_folder --lib
```

Expected: FAIL because `AppApi::sync_folder` does not exist.

- [ ] **Step 3: Add `sync_folder` and extract single-folder sync logic**

In `crates/app-api/src/lib.rs`, add this private struct near `SyncSummary`:

```rust
struct FolderSyncOutcome {
    message_count: u32,
    last_uid: Option<String>,
}
```

Add this public method after `sync_account`:

```rust
pub async fn sync_folder(&self, account_id: String, folder_id: String) -> ApiResult<SyncSummary> {
    let _guard = self.acquire_sync_lock(&account_id)?;
    let account = self.store.get_account(&account_id)?;
    if !account.sync_enabled {
        return Err(ApiError::InvalidRequest("account sync is disabled".to_string()));
    }
    let folder = self.store.get_folder(&folder_id)?;
    if folder.account_id != account.id {
        return Err(ApiError::InvalidRequest(
            "folder belongs to a different account".to_string(),
        ));
    }

    let settings = self.connection_settings_for_account(&account)?;
    let outcome = self.sync_one_folder(&settings, &account, &folder).await?;
    Ok(SyncSummary {
        account_id: account.id,
        folders: 1,
        messages: outcome.message_count,
        last_uid: outcome.last_uid,
        synced_at: now_rfc3339(),
    })
}
```

Extract the per-folder body from `sync_account_inner` into:

```rust
async fn sync_one_folder(
    &self,
    settings: &ConnectionSettings,
    account: &MailAccount,
    folder: &MailFolder,
) -> ApiResult<FolderSyncOutcome> {
    if folder.role == FolderRole::Sent {
        self.converge_fallback_sent_placeholders(&account.id, folder)?;
    }

    let previous_state = self.store.get_sync_state(&account.id, Some(&folder.id))?;
    if let Some(backoff_until) = previous_state
        .as_ref()
        .and_then(|state| state.backoff_until.as_deref())
    {
        if matches!(
            previous_state.as_ref().map(|state| state.state),
            Some(SyncStateKind::Backoff)
        ) && timestamp_is_future(backoff_until)
        {
            return Ok(FolderSyncOutcome {
                message_count: 0,
                last_uid: previous_state.and_then(|state| state.last_uid),
            });
        }
    }

    self.store.save_sync_state(&SyncState {
        account_id: account.id.clone(),
        folder_id: Some(folder.id.clone()),
        state: SyncStateKind::Syncing,
        last_uid: previous_state
            .as_ref()
            .and_then(|state| state.last_uid.clone()),
        last_synced_at: None,
        error_message: None,
        backoff_until: None,
        failure_count: previous_state
            .as_ref()
            .map(|state| state.failure_count)
            .unwrap_or(0),
    })?;

    let previous_last_uid = previous_state.and_then(|state| state.last_uid);
    let fetch_request = MessageFetchRequest {
        last_uid: None,
        limit: u32::MAX,
    };
    let messages = match self
        .protocol
        .fetch_messages(settings, account, folder, &fetch_request)
        .await
    {
        Ok(messages) => messages,
        Err(err) => {
            let failure_count = self
                .store
                .get_sync_state(&account.id, Some(&folder.id))?
                .map(|state| state.failure_count)
                .unwrap_or(0)
                .saturating_add(1);
            self.store.save_sync_state(&SyncState {
                account_id: account.id.clone(),
                folder_id: Some(folder.id.clone()),
                state: SyncStateKind::Backoff,
                last_uid: previous_last_uid,
                last_synced_at: None,
                error_message: Some(err.to_string()),
                backoff_until: Some(now_plus_seconds_rfc3339(backoff_seconds(failure_count))),
                failure_count,
            })?;
            return Err(ApiError::Protocol(err));
        }
    };

    let mut folder_last_uid = None;
    let mut remote_uids = HashSet::new();
    let mut message_count = 0_u32;
    for message in messages {
        folder_last_uid = message.uid.clone().or(folder_last_uid);
        if let Some(uid) = message.uid.as_ref() {
            remote_uids.insert(uid.clone());
        }
        self.store.upsert_message(&message)?;
        message_count += 1;
    }
    self.store
        .reconcile_folder_remote_uids(&account.id, &folder.id, &remote_uids)?;
    self.store.refresh_folder_counts(&folder.id)?;
    self.store.save_sync_state(&SyncState {
        account_id: account.id.clone(),
        folder_id: Some(folder.id.clone()),
        state: SyncStateKind::Idle,
        last_uid: folder_last_uid.clone(),
        last_synced_at: Some(now_rfc3339()),
        error_message: None,
        backoff_until: None,
        failure_count: 0,
    })?;

    Ok(FolderSyncOutcome {
        message_count,
        last_uid: folder_last_uid,
    })
}
```

Update `sync_account_inner` so its loop saves each folder, calls `sync_one_folder`, and accumulates `message_count`, `last_uid`, `attempted_folders`, `successful_folders`, and `first_folder_error` from the helper result. Keep the existing account-level backoff and final `SyncSummary` behavior.

- [ ] **Step 4: Verify backend folder sync**

Run:

```bash
cargo fmt --all --check
cargo test -p app-api sync_folder --lib
cargo test -p app-api --lib
cargo clippy -p app-api --all-targets -- -D warnings
```

Expected: all commands PASS.

- [ ] **Step 5: Commit backend folder sync**

Run:

```bash
git add crates/app-api/src/lib.rs
git commit -m "feat: add folder-level sync"
```

## Task 2: Extract Dynamic Tauri Watch Service

**Files:**
- Create: `src-tauri/src/watchers.rs`
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Create focused watcher module**

Create `src-tauri/src/watchers.rs` with the moved watcher types and functions from `main.rs`. The module must expose:

```rust
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use app_api::{ApiError, AppApi};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const MAIL_SYNC_EVENT: &str = "agentmail-mail-sync";
const WATCHER_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Clone, Default)]
pub struct WatcherRegistry {
    active: Arc<Mutex<HashSet<String>>>,
}

#[derive(Clone, Serialize)]
pub struct MailSyncEventPayload {
    pub account_id: String,
    pub folder_id: Option<String>,
    pub reason: &'static str,
    pub message: Option<String>,
}

fn to_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn watcher_key(account_id: &str, folder_id: &str) -> String {
    format!("{account_id}\0{folder_id}")
}

fn register_watcher(registry: &WatcherRegistry, account_id: &str, folder_id: &str) -> bool {
    let key = watcher_key(account_id, folder_id);
    match registry.active.lock() {
        Ok(mut active) => active.insert(key),
        Err(_) => false,
    }
}

fn remove_watcher(registry: &WatcherRegistry, account_id: &str, folder_id: &str) {
    let key = watcher_key(account_id, folder_id);
    if let Ok(mut active) = registry.active.lock() {
        active.remove(&key);
    }
}
```

Then move `start_account_watchers_for_api`, `start_folder_watcher`, and `emit_mail_sync_event` into this module. In `start_folder_watcher`, replace `api.sync_account(account_id.clone()).await` with:

```rust
api.sync_folder(account_id.clone(), folder_id.clone()).await
```

On `FolderWatchOutcome::Changed`, emit `reason: "watch_changed"` and `folder_id: Some(folder_id.clone())`. Do not call `start_account_watchers_for_api` recursively after every folder event; the loop re-enters IDLE for the same folder.

- [ ] **Step 2: Add registry unit tests**

At the bottom of `src-tauri/src/watchers.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_watcher_accepts_each_folder_once() {
        let registry = WatcherRegistry::default();

        assert!(register_watcher(&registry, "acct", "inbox"));
        assert!(!register_watcher(&registry, "acct", "inbox"));
        assert!(register_watcher(&registry, "acct", "trash"));
    }

    #[test]
    fn remove_watcher_allows_restart() {
        let registry = WatcherRegistry::default();

        assert!(register_watcher(&registry, "acct", "inbox"));
        remove_watcher(&registry, "acct", "inbox");

        assert!(register_watcher(&registry, "acct", "inbox"));
    }
}
```

- [ ] **Step 3: Update `main.rs` to use the watcher module**

At the top of `src-tauri/src/main.rs`, add:

```rust
mod watchers;
```

Replace the watcher imports with:

```rust
use watchers::{
    emit_mail_sync_event, start_account_watchers_for_api, MailSyncEventPayload, WatcherRegistry,
};
```

Remove these from `main.rs`:

- `collections::HashSet`
- `time::Duration`
- `serde::Serialize`
- `Emitter`
- inline `MAIL_SYNC_EVENT`
- inline `WATCHER_RETRY_DELAY`
- inline `WatcherRegistry`
- inline `MailSyncEventPayload`
- `watcher_key`
- `register_watcher`
- `remove_watcher`
- inline `start_account_watchers_for_api`
- inline `start_folder_watcher`
- inline `emit_mail_sync_event`

Keep the Tauri command `start_account_watchers(...)` in `main.rs`, but make it delegate to `watchers::start_account_watchers_for_api`.

- [ ] **Step 4: Verify watcher extraction**

Run:

```bash
cargo fmt --all --check
cargo test -p agentmail-app register_watcher
cargo check -p agentmail-app
cargo clippy -p app-api --all-targets -- -D warnings
```

Expected: commands PASS on a machine with Tauri native dependencies installed. If `cargo check -p agentmail-app` fails only because Linux GUI packages are missing, record the exact native dependency error and continue with the remaining repo checks.

- [ ] **Step 5: Commit watcher extraction**

Run:

```bash
git add src-tauri/src/main.rs src-tauri/src/watchers.rs
git commit -m "refactor: extract qq idle watcher service"
```

## Task 3: Remove Frontend Automatic Polling

**Files:**
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/App.test.ts`
- Modify: `ui/src/lib/syncFlows.ts`

- [ ] **Step 1: Update tests to remove automatic sync helper coverage**

In `ui/src/App.test.ts`, remove `runAutomaticAccountSync` from the `./lib/syncFlows` import and delete the whole `describe("runAutomaticAccountSync", ...)` block.

Run:

```bash
pnpm test App.test.ts
```

Expected: PASS after deleting tests, because automatic background sync behavior is no longer a requirement.

- [ ] **Step 2: Remove automatic polling code**

In `ui/src/App.tsx`, remove:

- `AUTO_SYNC_INTERVAL_MS`
- `isAutoSyncRunningRef`
- `runAutomaticAccountSync` import
- the `useEffect` that calls `window.setInterval(runAutoSync, AUTO_SYNC_INTERVAL_MS)`

In `ui/src/lib/syncFlows.ts`, remove:

- `AutomaticAccountSyncRequest`
- `runAutomaticAccountSync`

- [ ] **Step 3: Verify no automatic polling remains**

Run:

```bash
rg -n "AUTO_SYNC_INTERVAL_MS|runAutomaticAccountSync|setInterval\\(runAutoSync|auto sync complete" ui/src
pnpm test App.test.ts
pnpm build
```

Expected: `rg` returns no matches, and both pnpm commands PASS.

- [ ] **Step 4: Commit automatic polling removal**

Run:

```bash
git add ui/src/App.tsx ui/src/App.test.ts ui/src/lib/syncFlows.ts
git commit -m "refactor: remove automatic polling sync"
```

## Task 4: Add Manual Sync Waiting Ring

**Files:**
- Create: `ui/src/lib/syncUi.ts`
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/App.test.ts`
- Modify: `ui/src/styles/app.css`

- [ ] **Step 1: Add failing UI state helper tests**

In `ui/src/App.test.ts`, import:

```ts
import { getManualSyncButtonState } from "./lib/syncUi";
```

Add:

```ts
describe("getManualSyncButtonState", () => {
  it("disables manual sync when no account is selected", () => {
    expect(getManualSyncButtonState(null, false)).toEqual({
      className: "icon-button sync-button",
      disabled: true,
      title: "Sync account"
    });
  });

  it("shows a pending state while manual sync is running", () => {
    expect(getManualSyncButtonState("acct-1", true)).toEqual({
      className: "icon-button sync-button syncing",
      disabled: true,
      title: "Sync running"
    });
  });
});
```

Run:

```bash
pnpm test App.test.ts
```

Expected: FAIL because `ui/src/lib/syncUi.ts` does not exist.

- [ ] **Step 2: Create sync UI helper**

Create `ui/src/lib/syncUi.ts`:

```ts
export function getManualSyncButtonState(selectedAccountId: string | null, isManualSyncing: boolean) {
  return {
    className: isManualSyncing ? "icon-button sync-button syncing" : "icon-button sync-button",
    disabled: !selectedAccountId || isManualSyncing,
    title: isManualSyncing ? "Sync running" : "Sync account"
  };
}
```

- [ ] **Step 3: Wire manual sync loading state in `App.tsx`**

In `ui/src/App.tsx`, import:

```ts
import { getManualSyncButtonState } from "./lib/syncUi";
```

Add state near other UI state:

```ts
const [isManualSyncing, setManualSyncing] = useState(false);
```

Update `handleSync`:

```ts
const handleSync = useCallback(async () => {
  if (!selectedAccountId || isManualSyncing) return;
  setManualSyncing(true);
  setStatus("sync running");
  try {
    setStatus(
      await runManualAccountSync({
        accountId: selectedAccountId,
        folderId: selectedFolderId,
        query,
        syncAccount: api.syncAccount,
        startAccountWatchers: api.startAccountWatchers,
        refreshFolders,
        refreshMessages,
        refreshSyncState,
        refreshAudits
      })
    );
  } catch (error) {
    await refreshSyncState(selectedAccountId);
    await refreshAudits();
    setStatus(`sync failed: ${String(error)}`);
  } finally {
    setManualSyncing(false);
  }
}, [
  isManualSyncing,
  query,
  refreshAudits,
  refreshFolders,
  refreshMessages,
  refreshSyncState,
  selectedAccountId,
  selectedFolderId
]);
```

Before returning JSX, add:

```ts
const manualSyncButton = getManualSyncButtonState(selectedAccountId, isManualSyncing);
```

Update the sync button:

```tsx
<button
  className={manualSyncButton.className}
  type="button"
  onClick={handleSync}
  disabled={manualSyncButton.disabled}
  title={manualSyncButton.title}
>
  <RefreshCcw size={17} />
</button>
```

- [ ] **Step 4: Add waiting ring CSS**

In `ui/src/styles/app.css`, near `.icon-button` styles, add:

```css
.sync-button.syncing {
  cursor: progress;
}

.sync-button.syncing svg {
  animation: sync-spin 0.9s linear infinite;
}

@keyframes sync-spin {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}
```

- [ ] **Step 5: Verify manual sync UI**

Run:

```bash
pnpm test App.test.ts
pnpm build
```

Expected: both commands PASS.

- [ ] **Step 6: Commit manual sync waiting ring**

Run:

```bash
git add ui/src/lib/syncUi.ts ui/src/App.tsx ui/src/App.test.ts ui/src/styles/app.css
git commit -m "feat: show manual sync waiting state"
```

## Task 5: Update Docs And Run Full Verification

**Files:**
- Modify: `docs/REAL_MAIL_ACCEPTANCE.md`
- Modify: `docs/PROJECT_STATUS.md`

- [ ] **Step 1: Update acceptance docs**

In `docs/REAL_MAIL_ACCEPTANCE.md`, add QQ IDLE acceptance bullets:

```md
- QQ automatic sync is IMAP IDLE-driven for all selectable folders returned by the account's IMAP folder list.
- Background automatic sync must not use a 30-second polling interval.
- Manual sync remains available from the topbar, shows a waiting ring while active, and refreshes folders/messages/sync state/audits after completion.
```

In `docs/PROJECT_STATUS.md`, update the sync section to state:

```md
- QQ Mail automatic refresh uses IMAP IDLE watchers for the account's stored selectable folders.
- Manual sync is still available and reconciles watchers after an explicit account sync.
- Folder create/delete discovery is not automatic in this stage; startup, account save, or manual sync refreshes the folder list.
```

- [ ] **Step 2: Run full verification**

Run:

```bash
cargo fmt --all --check
cargo clippy -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api --all-targets -- -D warnings
cargo test -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api
pnpm test
pnpm build
pnpm rust:check
git diff --check
```

Also run:

```bash
cargo check -p agentmail-app
```

Expected: all commands PASS where the local machine has Tauri native GUI dependencies. If `cargo check -p agentmail-app` is blocked by missing Linux GUI packages, record the blocker and verify the Rust workspace and frontend commands above are clean.

- [ ] **Step 3: Review final diff**

Run:

```bash
git status --short --branch
git log --oneline --decorate -10
rg -n "AUTO_SYNC_INTERVAL_MS|runAutomaticAccountSync|setInterval\\(runAutoSync|auto sync complete" ui/src
```

Expected:

- Only intended source/docs changes are uncommitted before the docs commit.
- `rg` returns no matches in `ui/src`.
- No Gmail-specific behavior was added.
- No folder-name whitelist was added for QQ watcher selection.

- [ ] **Step 4: Commit docs**

Run:

```bash
git add docs/REAL_MAIL_ACCEPTANCE.md docs/PROJECT_STATUS.md
git commit -m "docs: update qq idle sync acceptance"
```

## Task 6: Final Review And Handoff

**Files:**
- No source edits unless review finds a concrete regression.

- [ ] **Step 1: Run final review checks**

Run:

```bash
git status --short --branch
git log --oneline --decorate -12
git diff --stat origin/main..HEAD
```

Review for:

- QQ-only scope.
- Dynamic folder list from stored selectable folders.
- No frontend background polling.
- Manual sync button remains and shows a waiting state.
- IDLE event path uses `sync_folder`, not full `sync_account`.
- Single watcher failure remains folder-scoped.

- [ ] **Step 2: Request code review before merge/push**

Use `superpowers:requesting-code-review` after implementation. The review context should cite this plan, the design spec, `origin/main` as base, and `HEAD` as the implementation result.

- [ ] **Step 3: Finish branch**

After code review issues are resolved and verification is fresh, use `superpowers:finishing-a-development-branch`. If the user requests direct completion, commit any remaining changes, merge if needed, and push `main`.
