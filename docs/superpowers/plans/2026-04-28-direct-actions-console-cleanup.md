# Direct Actions Console Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the visible and behavioral pending-action confirmation flow, execute mail sends and normalized mail actions directly, remove the sync footer panel, and make the activity log a hidden-by-default display setting.

**Architecture:** Keep the existing SQLite `pending_actions` table for compatibility, but stop new product flows from writing or reading it. Direct sends go through SMTP first and then persist a local Sent row plus an executed audit; direct mail actions keep the current normalization rules and call the existing execution path immediately. The React UI removes pending-action state and footer panels, while adding a localStorage-backed activity log visibility setting in the existing `DISPLAY` tab.

**Tech Stack:** Rust workspace (`mail-core`, `mail-store`, `app-api`, `mail-protocol`, `ai-remote`), Tauri v2 commands, React/Vite TypeScript UI, SQLite, Vitest, Cargo tests, pnpm.

---

## File Structure

- Modify `crates/mail-store/src/lib.rs`: add one transaction helper for direct Sent message plus audit persistence, and add a focused unit test.
- Modify `crates/app-api/src/lib.rs`: change direct action execution and direct send behavior; update app-api tests around send, Trash delete, batch actions, and existing Sent reconciliation.
- Modify `ui/src/api.ts`: remove pending-action frontend types and command wrappers.
- Modify `ui/src/App.tsx`: remove pending-action state/handlers/component, remove pending refresh arguments from sync helpers, add send status and activity-log visibility helpers, and add the setting-controlled footer.
- Modify `ui/src/App.test.ts`: update helper tests to match direct send and no pending refresh.
- Modify `ui/src/data/demoBackend.ts`: make browser demo actions and sends execute directly.
- Modify `ui/src/api.test.ts`: update demo send test to direct-send behavior.
- Modify `ui/src/styles/app.css`: remove reserved footer space by default and support an activity-log-visible footer row.
- Modify `README.md`, `docs/PROJECT_STATUS.md`, `docs/DECISIONS.md`, `docs/NEXT_STEPS.md`, `docs/REAL_MAIL_ACCEPTANCE.md`: align handoff and acceptance text with direct send and optional activity log display.

## Task 1: Store Transaction For Direct Sent Persistence

**Files:**
- Modify: `crates/mail-store/src/lib.rs`
- Test: `crates/mail-store/src/lib.rs`

- [ ] **Step 1: Add a failing store test**

Insert this test in `crates/mail-store/src/lib.rs` inside `mod tests`, after `pending_actions_round_trip_and_filter_to_pending`.

```rust
#[test]
fn direct_sent_message_persists_message_counts_and_audit() {
    let store = MailStore::memory().unwrap();
    let now = now_rfc3339();
    let account = MailAccount {
        id: "acct".to_string(),
        display_name: "Ops".to_string(),
        email: "ops@example.com".to_string(),
        imap_host: "imap.example.com".to_string(),
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.example.com".to_string(),
        smtp_port: 465,
        smtp_tls: true,
        sync_enabled: true,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    store.save_account(&account).unwrap();

    let sent = MailFolder {
        id: "acct:sent".to_string(),
        account_id: account.id.clone(),
        name: "Sent".to_string(),
        path: "Sent".to_string(),
        role: FolderRole::Sent,
        unread_count: 0,
        total_count: 0,
    };
    let message = MailMessage {
        id: "local-sent-1".to_string(),
        account_id: account.id.clone(),
        folder_id: sent.id.clone(),
        uid: None,
        message_id_header: Some("<local-sent-1@agentmail.local>".to_string()),
        subject: "Direct sent".to_string(),
        sender: account.email.clone(),
        recipients: vec!["sec@example.com".to_string()],
        cc: vec!["ops-lead@example.com".to_string()],
        received_at: now.clone(),
        body_preview: "Direct body".to_string(),
        body: Some("Direct body".to_string()),
        attachments: Vec::new(),
        flags: MessageFlags {
            is_read: true,
            is_starred: false,
            is_answered: true,
            is_forwarded: false,
        },
        size_bytes: None,
        deleted_at: None,
    };
    let audit = MailActionAudit {
        id: "audit-1".to_string(),
        account_id: account.id.clone(),
        action: MailActionKind::Send,
        message_ids: vec![message.id.clone()],
        status: ActionAuditStatus::Executed,
        error_message: None,
        created_at: now,
    };

    store
        .save_direct_sent_message(&sent, &message, &audit)
        .unwrap();

    let saved_sent = store.get_folder(&sent.id).unwrap();
    assert_eq!(saved_sent.total_count, 1);
    assert_eq!(saved_sent.unread_count, 0);

    let messages = store
        .list_messages(&MessageQuery {
            account_id: Some(account.id.clone()),
            folder_id: Some(sent.id.clone()),
            limit: 10,
            offset: 0,
        })
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, message.id);
    assert_eq!(messages[0].message_id_header.as_deref(), Some("<local-sent-1@agentmail.local>"));

    let audits = store.list_audits(5).unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].status, ActionAuditStatus::Executed);
    assert_eq!(audits[0].message_ids, vec!["local-sent-1".to_string()]);
}
```

- [ ] **Step 2: Run the failing store test**

Run:

```bash
cargo test -p mail-store direct_sent_message_persists_message_counts_and_audit
```

Expected: FAIL with an error that `save_direct_sent_message` is not found for `MailStore`.

- [ ] **Step 3: Add the store helper**

Add this method in the `impl MailStore` block in `crates/mail-store/src/lib.rs`, directly after the existing queued-send transaction helper.

```rust
pub fn save_direct_sent_message(
    &self,
    sent_folder: &MailFolder,
    message: &MailMessage,
    audit: &MailActionAudit,
) -> StoreResult<()> {
    let mut conn = self.conn.lock();
    let tx = conn.transaction()?;
    save_folder_on_conn(&tx, sent_folder)?;
    upsert_message_tx(&tx, message)?;
    refresh_folder_counts_on_conn(&tx, &sent_folder.id)?;
    write_audit_on_conn(&tx, audit)?;
    tx.commit()?;
    Ok(())
}
```

- [ ] **Step 4: Run the store test again**

Run:

```bash
cargo test -p mail-store direct_sent_message_persists_message_counts_and_audit
```

Expected: PASS.

- [ ] **Step 5: Commit the store helper**

Run:

```bash
git add crates/mail-store/src/lib.rs
git commit -m "feat: add direct sent persistence"
```

## Task 2: App API Direct Send And Direct Actions

**Files:**
- Modify: `crates/app-api/src/lib.rs`
- Test: `crates/app-api/src/lib.rs`

- [ ] **Step 1: Replace send queue tests with direct-send tests**

In `crates/app-api/src/lib.rs`, remove the obsolete send-queue test block that starts at `send_message_queues_until_confirmed` and ends at `reject_pending_action_does_not_execute_protocol`.

Keep `delete_uidless_sent_placeholder_soft_deletes_local_message` if it still compiles after `send_message` returns a local Sent message id; update its setup to use that returned id directly instead of loading a pending row.

In that retained test, replace its send setup with:

```rust
let local_message_id = api
    .send_message(SendMessageDraft {
        account_id: account.id.clone(),
        to: vec!["sec@example.com".to_string()],
        cc: Vec::new(),
        subject: "Delete local sent message".to_string(),
        body: "body".to_string(),
        message_id_header: None,
    })
    .await
    .unwrap();
```

Insert these tests in the same test module near the removed send tests.

```rust
#[tokio::test]
async fn send_message_executes_immediately_and_records_sent_copy() {
    let protocol = RecordingProtocol::default();
    let sends = Arc::clone(&protocol.sends);
    let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
    let account = add_sample_account(&api).await;
    let sent = save_sent_folder(&api, &account);

    let sent_message_id = api
        .send_message(SendMessageDraft {
            account_id: account.id.clone(),
            to: vec!["sec@example.com".to_string()],
            cc: vec!["ops-lead@example.com".to_string()],
            subject: "Direct send".to_string(),
            body: "body\nwith local visibility".to_string(),
            message_id_header: None,
        })
        .await
        .unwrap();

    let sends = sends.lock();
    assert_eq!(sends.len(), 1);
    let generated_header = sends[0].message_id_header.clone().unwrap();
    assert!(generated_header.starts_with('<'));
    assert!(generated_header.ends_with("@agentmail.local>"));
    drop(sends);

    assert!(api
        .list_pending_actions(Some(account.id.clone()))
        .unwrap()
        .is_empty());

    let sent_messages = api
        .list_messages(MessageQuery {
            account_id: Some(account.id.clone()),
            folder_id: Some(sent.id.clone()),
            limit: 10,
            offset: 0,
        })
        .unwrap();
    assert_eq!(sent_messages.len(), 1);
    assert_eq!(sent_messages[0].id, sent_message_id);
    assert_eq!(sent_messages[0].uid, None);
    assert_eq!(sent_messages[0].sender, account.email);
    assert_eq!(sent_messages[0].recipients, vec!["sec@example.com"]);
    assert_eq!(sent_messages[0].cc, vec!["ops-lead@example.com"]);
    assert_eq!(sent_messages[0].subject, "Direct send");
    assert_eq!(sent_messages[0].body.as_deref(), Some("body\nwith local visibility"));
    assert_eq!(sent_messages[0].message_id_header.as_deref(), Some(generated_header.as_str()));
    assert!(sent_messages[0].flags.is_read);
    assert!(sent_messages[0].flags.is_answered);

    let updated_sent = api.store.get_folder(&sent.id).unwrap();
    assert_eq!(updated_sent.total_count, 1);
    assert_eq!(updated_sent.unread_count, 0);

    let audits = api.get_audit_log(Some(5)).unwrap();
    assert_eq!(audits[0].action, MailActionKind::Send);
    assert_eq!(audits[0].status, ActionAuditStatus::Executed);
    assert_eq!(audits[0].message_ids, vec![sent_message_id]);
}

#[tokio::test]
async fn send_message_creates_fallback_sent_folder_for_direct_send() {
    let protocol = RecordingProtocol::default();
    let sends = Arc::clone(&protocol.sends);
    let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
    let account = add_sample_account(&api).await;

    let sent_message_id = api
        .send_message(SendMessageDraft {
            account_id: account.id.clone(),
            to: vec!["sec@example.com".to_string()],
            cc: Vec::new(),
            subject: "Fallback sent".to_string(),
            body: "body".to_string(),
            message_id_header: None,
        })
        .await
        .unwrap();

    assert_eq!(sends.lock().len(), 1);
    let sent = api
        .store
        .find_folder_by_role(&account.id, FolderRole::Sent)
        .unwrap()
        .unwrap();
    assert_eq!(sent.id, format!("{}:sent", account.id));
    assert_eq!(sent.path, "Sent");
    assert_eq!(sent.total_count, 1);
    assert!(api.store.get_message(&sent_message_id).is_ok());
}

#[tokio::test]
async fn failed_direct_send_records_failed_audit_without_sent_copy() {
    let protocol = RecordingProtocol {
        fail_sends: true,
        ..RecordingProtocol::default()
    };
    let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
    let account = add_sample_account(&api).await;
    let sent = save_sent_folder(&api, &account);

    let result = api
        .send_message(SendMessageDraft {
            account_id: account.id.clone(),
            to: vec!["sec@example.com".to_string()],
            cc: Vec::new(),
            subject: "Fail send".to_string(),
            body: "body".to_string(),
            message_id_header: None,
        })
        .await;

    assert!(result.is_err());
    assert!(api
        .list_messages(MessageQuery {
            account_id: Some(account.id.clone()),
            folder_id: Some(sent.id.clone()),
            limit: 10,
            offset: 0,
        })
        .unwrap()
        .is_empty());
    assert_eq!(api.store.get_folder(&sent.id).unwrap().total_count, 0);

    let audits = api.get_audit_log(Some(5)).unwrap();
    assert_eq!(audits[0].action, MailActionKind::Send);
    assert_eq!(audits[0].status, ActionAuditStatus::Failed);
    assert!(audits[0]
        .error_message
        .as_deref()
        .unwrap()
        .contains("forced send failure"));
}
```

- [ ] **Step 2: Change the Trash delete test expectation**

In `permanent_delete_from_trash_refetches_uid_before_remote_delete`, replace the pending-action expectation block with this direct execution assertion.

```rust
assert_eq!(result.kind, MailActionResultKind::Executed);
assert_eq!(result.pending_action_id, None);

let recorded = actions.lock();
assert_eq!(recorded.len(), 2);
assert_eq!(recorded[1].action, MailActionKind::PermanentDelete);
assert_eq!(recorded[1].source_folder.role, FolderRole::Trash);
assert_eq!(recorded[1].target_folder, None);
assert_eq!(recorded[1].uids, vec!["900".to_string()]);
drop(recorded);

assert!(api
    .store
    .get_message(&message.id)
    .unwrap()
    .deleted_at
    .is_some());
```

- [ ] **Step 3: Add a batch normalization test**

Insert this app-api test near the other action tests.

```rust
#[tokio::test]
async fn batch_delete_executes_directly_after_normalization() {
    let protocol = RecordingProtocol::default();
    let actions = Arc::clone(&protocol.actions);
    let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
    let account = add_sample_account(&api).await;
    api.sync_account(account.id.clone()).await.unwrap();
    let message = first_message(&api, &account.id);
    let second = MailMessage {
        id: format!("{}:second", message.id),
        uid: Some("43".to_string()),
        message_id_header: Some("<43@example.com>".to_string()),
        subject: "Second action".to_string(),
        body_preview: "second".to_string(),
        body: Some("second".to_string()),
        ..message.clone()
    };
    api.store.upsert_message(&second).unwrap();

    let result = api
        .execute_mail_action(MailActionRequest {
            action: MailActionKind::Delete,
            account_id: account.id.clone(),
            message_ids: vec![message.id.clone(), second.id.clone()],
            target_folder_id: None,
        })
        .await
        .unwrap();

    assert_eq!(result.kind, MailActionResultKind::Executed);
    assert_eq!(result.pending_action_id, None);
    let recorded = actions.lock();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].action, MailActionKind::BatchDelete);
    assert_eq!(recorded[0].uids, vec!["42".to_string(), "43".to_string()]);
}
```

- [ ] **Step 4: Run the failing app-api tests**

Run:

```bash
cargo test -p app-api send_message_executes_immediately_and_records_sent_copy
cargo test -p app-api send_message_creates_fallback_sent_folder_for_direct_send
cargo test -p app-api failed_direct_send_records_failed_audit_without_sent_copy
cargo test -p app-api batch_delete_executes_directly_after_normalization
cargo test -p app-api permanent_delete_from_trash_refetches_uid_before_remote_delete
```

Expected: FAIL because `send_message` still returns pending ids, `execute_mail_action` still returns pending for normalized destructive actions, and removed tests still reference old behavior until the test block is edited.

- [ ] **Step 5: Change `execute_mail_action` to execute normalized actions directly**

Replace `AppApi::execute_mail_action` with this implementation.

```rust
pub async fn execute_mail_action(
    &self,
    request: MailActionRequest,
) -> ApiResult<MailActionResult> {
    let normalized_action = self.direct_action_for_request(&request)?;
    if matches!(normalized_action, MailActionKind::Send | MailActionKind::Forward) {
        return Err(ApiError::InvalidRequest(format!(
            "{normalized_action:?} is not implemented through execute_mail_action"
        )));
    }

    let request = MailActionRequest {
        action: normalized_action,
        ..request
    };
    self.execute_confirmed_mail_action(&request).await?;
    Ok(MailActionResult {
        kind: MailActionResultKind::Executed,
        pending_action_id: None,
    })
}
```

Rename `confirmation_action_for_request` to `direct_action_for_request` and keep its body unchanged.

```rust
fn direct_action_for_request(
    &self,
    request: &MailActionRequest,
) -> ApiResult<MailActionKind> {
    match request.action {
        MailActionKind::Delete => {
            if request.message_ids.is_empty() {
                return Ok(MailActionKind::Delete);
            }
            let first_message = self.store.get_message(&request.message_ids[0])?;
            if first_message.account_id != request.account_id {
                return Err(ApiError::InvalidRequest(format!(
                    "message {} does not belong to account {}",
                    first_message.id, request.account_id
                )));
            }
            let source_folder = self.store.get_folder(&first_message.folder_id)?;
            if source_folder.role == FolderRole::Trash {
                Ok(MailActionKind::PermanentDelete)
            } else if request.message_ids.len() > 1 {
                Ok(MailActionKind::BatchDelete)
            } else {
                Ok(MailActionKind::Delete)
            }
        }
        MailActionKind::Move if request.message_ids.len() > 1 => Ok(MailActionKind::BatchMove),
        action => Ok(action),
    }
}
```

- [ ] **Step 6: Change `send_message` to send immediately**

Replace `AppApi::send_message` with this implementation.

```rust
pub async fn send_message(&self, draft: SendMessageDraft) -> ApiResult<String> {
    if draft.to.is_empty() {
        return Err(ApiError::InvalidRequest(
            "recipient list is empty".to_string(),
        ));
    }
    let account = self.store.get_account(&draft.account_id)?;
    let sent = self.sent_folder_for_local_send(&account)?;
    let now = now_rfc3339();
    let message_id = new_id();
    let message_id_header = draft
        .message_id_header
        .clone()
        .unwrap_or_else(|| format!("<{message_id}@agentmail.local>"));
    let draft = SendMessageDraft {
        message_id_header: Some(message_id_header.clone()),
        ..draft
    };

    if let Err(err) = self.send_message_now(&draft).await {
        self.write_audit(
            &account.id,
            MailActionKind::Send,
            Vec::new(),
            ActionAuditStatus::Failed,
            Some(err.to_string()),
        )?;
        return Err(err);
    }

    let sent_message = MailMessage {
        id: message_id.clone(),
        account_id: account.id.clone(),
        folder_id: sent.id.clone(),
        uid: None,
        message_id_header: Some(message_id_header),
        subject: draft.subject.clone(),
        sender: account.email.clone(),
        recipients: draft.to.clone(),
        cc: draft.cc.clone(),
        received_at: now.clone(),
        body_preview: body_preview_from_body(&draft.body),
        body: Some(draft.body.clone()),
        attachments: Vec::new(),
        flags: mail_core::MessageFlags {
            is_read: true,
            is_starred: false,
            is_answered: true,
            is_forwarded: false,
        },
        size_bytes: None,
        deleted_at: None,
    };
    let audit = MailActionAudit {
        id: new_id(),
        account_id: account.id.clone(),
        action: MailActionKind::Send,
        message_ids: vec![message_id.clone()],
        status: ActionAuditStatus::Executed,
        error_message: None,
        created_at: now,
    };

    self.store
        .save_direct_sent_message(&sent, &sent_message, &audit)?;
    if account.sync_enabled {
        let _ = self.reconcile_sent_placeholders_after_send(&account.id).await;
    }
    Ok(message_id)
}
```

- [ ] **Step 7: Run app-api tests**

Run:

```bash
cargo test -p app-api
```

Expected: PASS after old pending-send tests are removed or rewritten and direct behavior is implemented.

- [ ] **Step 8: Commit app-api direct behavior**

Run:

```bash
git add crates/app-api/src/lib.rs
git commit -m "feat: execute mail actions directly"
```

## Task 3: Frontend API And Sync Helper Cleanup

**Files:**
- Modify: `ui/src/api.ts`
- Modify: `ui/src/App.tsx`
- Test: `ui/src/App.test.ts`

- [ ] **Step 1: Update helper tests for direct send and no pending refresh**

In `ui/src/App.test.ts`, update the import list to include new helpers and remove pending send naming.

```ts
import {
  ACTIVITY_LOG_STORAGE_KEY,
  applyThemeModeToDocument,
  clampWorkspaceSplitPercent,
  formatAuditLine,
  formatFolderCount,
  formatSendStatus,
  getAppShellClassName,
  getWorkspaceSplitModel,
  getNextThemeMode,
  readStoredActivityLogVisibility,
  readStoredWorkspaceSplitPercent,
  readStoredThemeMode,
  refreshAfterMailSyncEvent,
  runAutomaticAccountSync,
  runInitialAccountSync,
  runManualAccountSync,
  WORKSPACE_SPLIT_STORAGE_KEY,
  THEME_MODE_STORAGE_KEY
} from "./App";
```

Replace the `formatSendQueuedStatus` test block with:

```ts
describe("formatSendStatus", () => {
  it("states that sends execute directly", () => {
    expect(formatSendStatus(["ops@example.com"])).toBe("sent to ops@example.com");
  });
});
```

Add this test block after the theme mode tests.

```ts
describe("activity log visibility helpers", () => {
  it("defaults to hidden when no saved preference exists", () => {
    window.localStorage.removeItem(ACTIVITY_LOG_STORAGE_KEY);

    expect(readStoredActivityLogVisibility(window.localStorage)).toBe(false);
  });

  it("reads a saved visible preference", () => {
    window.localStorage.setItem(ACTIVITY_LOG_STORAGE_KEY, "true");

    expect(readStoredActivityLogVisibility(window.localStorage)).toBe(true);
  });

  it("keeps the footer class out of the default app shell", () => {
    expect(getAppShellClassName(false)).toBe("app-shell");
    expect(getAppShellClassName(true)).toBe("app-shell activity-log-visible");
  });
});
```

In every sync helper test, remove `refreshPendingActions` setup, request property, and assertion. For example, update the first `runInitialAccountSync` test setup to:

```ts
const refreshFolders = vi.fn().mockResolvedValue(undefined);
const refreshMessages = vi.fn().mockResolvedValue(undefined);
const refreshSyncState = vi.fn().mockResolvedValue(undefined);
const refreshAudits = vi.fn().mockResolvedValue(undefined);
const startAccountWatchers = vi.fn().mockResolvedValue(undefined);

const status = await runInitialAccountSync({
  accountId: "acct-1",
  email: "ops@example.com",
  folderId: "previous-folder",
  query: "release",
  syncAccount: vi.fn().mockResolvedValue({
    account_id: "acct-1",
    folders: 4,
    messages: 18,
    synced_at: "2026-04-27T00:00:00Z"
  }),
  startAccountWatchers,
  refreshFolders,
  refreshMessages,
  refreshSyncState,
  refreshAudits
});
```

- [ ] **Step 2: Run failing frontend tests**

Run:

```bash
pnpm test -- App.test.ts
```

Expected: FAIL because the new helpers do not exist and sync helper types still require `refreshPendingActions`.

- [ ] **Step 3: Remove pending frontend API bindings**

In `ui/src/api.ts`, remove the `PendingMailAction` interface and these command entries from `CommandMap`.

```ts
list_pending_actions: PendingMailAction[];
confirm_action: MailActionResult;
reject_action: null;
```

Remove these wrappers from `api`.

```ts
listPendingActions: (accountId?: string | null) => call("list_pending_actions", { accountId: accountId ?? null }),
confirmAction: (actionId: string) => call("confirm_action", { actionId }),
rejectAction: (actionId: string) => call("reject_action", { actionId }),
```

- [ ] **Step 4: Add activity visibility and direct-send helpers**

In `ui/src/App.tsx`, remove `PendingMailAction` from imports and replace `formatSendQueuedStatus` with these exports.

```ts
export const ACTIVITY_LOG_STORAGE_KEY = "agentmail-show-activity-log";

export function formatSendStatus(recipients: string[]) {
  return `sent to ${recipients.join(", ")}`;
}

export function readStoredActivityLogVisibility(storage: Pick<Storage, "getItem"> | null | undefined) {
  try {
    return storage?.getItem(ACTIVITY_LOG_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

function writeStoredActivityLogVisibility(storage: Pick<Storage, "setItem"> | null | undefined, visible: boolean) {
  try {
    storage?.setItem(ACTIVITY_LOG_STORAGE_KEY, visible ? "true" : "false");
  } catch {
    // Storage can be unavailable in hardened desktop/webview contexts.
  }
}

export function getAppShellClassName(showActivityLog: boolean) {
  return showActivityLog ? "app-shell activity-log-visible" : "app-shell";
}
```

- [ ] **Step 5: Remove pending refresh from sync helper types and bodies**

In `ui/src/App.tsx`, remove `refreshPendingActions` from `InitialAccountSyncRequest`, `RefreshAfterMailSyncEventRequest`, `AutomaticAccountSyncRequest`, and `ManualAccountSyncRequest`.

Each helper should refresh the same four surfaces. For `runAutomaticAccountSync`, the settled refresh block should become:

```ts
await Promise.allSettled([
  refreshFolders(selectedAccountId),
  refreshMessages(selectedAccountId, selectedFolderId, query),
  refreshSyncState(selectedAccountId),
  refreshAudits()
]);
```

Apply the same pattern to `runInitialAccountSync`, `refreshAfterMailSyncEvent`, and `runManualAccountSync`, using their local account id variables.

- [ ] **Step 6: Remove pending state and handlers from `App`**

In `ui/src/App.tsx`, remove:

```ts
const [pendingActions, setPendingActions] = useState<PendingMailAction[]>([]);
```

Remove this callback:

```ts
const refreshPendingActions = useCallback(async (accountId: string | null) => {
  setPendingActions(await api.listPendingActions(accountId));
}, []);
```

Remove `refreshPendingActions` from every dependency array and from every call to `runInitialAccountSync`, `refreshAfterMailSyncEvent`, `runAutomaticAccountSync`, and `runManualAccountSync`.

Remove the complete `handleConfirmPending` and `handleRejectPending` callbacks.

- [ ] **Step 7: Update send success handling**

Replace `handleSent` in `ui/src/App.tsx` with:

```ts
const handleSent = useCallback(
  async (draft: SendMessageDraft) => {
    try {
      await api.sendMessage(draft);
      await refreshFolders(draft.account_id);
      await refreshMessages(draft.account_id, selectedFolderId, query);
      await refreshAudits();
      setComposerOpen(false);
      setStatus(formatSendStatus(draft.to));
    } catch (error) {
      await refreshAudits();
      setStatus(`send failed: ${String(error)}`);
      throw error;
    }
  },
  [query, refreshAudits, refreshFolders, refreshMessages, selectedFolderId]
);
```

- [ ] **Step 8: Run frontend helper tests**

Run:

```bash
pnpm test -- App.test.ts
```

Expected: PASS after pending refresh and helper updates are complete.

- [ ] **Step 9: Commit API/helper cleanup**

Run:

```bash
git add ui/src/api.ts ui/src/App.tsx ui/src/App.test.ts
git commit -m "feat: remove pending action frontend state"
```

## Task 4: Activity Log Toggle And Layout Cleanup

**Files:**
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/styles/app.css`
- Test: `ui/src/App.test.ts`

- [ ] **Step 1: Add activity log state to `App`**

In `ui/src/App.tsx`, add this state near `themeMode`.

```ts
const [showActivityLog, setShowActivityLog] = useState(() =>
  typeof window === "undefined" ? false : readStoredActivityLogVisibility(window.localStorage)
);
```

Add this effect after the theme mode effect.

```ts
useEffect(() => {
  writeStoredActivityLogVisibility(typeof window === "undefined" ? null : window.localStorage, showActivityLog);
}, [showActivityLog]);
```

Change the root element to:

```tsx
<main className={getAppShellClassName(showActivityLog)}>
```

- [ ] **Step 2: Replace footer rendering**

In `ui/src/App.tsx`, replace the whole current `<footer className="status-console">...</footer>` block with:

```tsx
{showActivityLog ? (
  <footer className="status-console">
    <section className="console-panel audit-feed">
      <header>AUDIT / ACTIVITY LOG</header>
      <p>{accountSyncState?.error_message ?? status}</p>
      {audits.slice(0, 8).map((audit) => (
        <code key={audit.id}>{formatAuditLine(audit)}</code>
      ))}
    </section>
  </footer>
) : null}
```

Remove the `PendingActionQueueProps` interface and the `PendingActionQueue` component from the bottom of `ui/src/App.tsx`.

- [ ] **Step 3: Pass activity-log settings into configuration**

Extend `ConfigurationModalProps` in `ui/src/App.tsx`.

```ts
interface ConfigurationModalProps {
  accounts: MailAccount[];
  selectedAccountId: string | null;
  settings: AiSettingsView | null;
  themeMode: ThemeMode;
  showActivityLog: boolean;
  onThemeModeChange: (mode: ThemeMode) => void;
  onShowActivityLogChange: (visible: boolean) => void;
  onClose: () => void;
  onAccountSaved: (account: MailAccount) => Promise<void>;
  onAiSettingsSaved: () => Promise<void>;
}
```

Pass the props from the `ConfigurationModal` call site:

```tsx
showActivityLog={showActivityLog}
onShowActivityLogChange={setShowActivityLog}
```

Add them to the function parameter destructuring:

```ts
showActivityLog,
onThemeModeChange,
onShowActivityLogChange,
```

- [ ] **Step 4: Add the display toggle**

In the `DISPLAY` tab branch of `ConfigurationModal`, keep the existing theme switch row and add this second row below it.

```tsx
<div className="theme-switch-row">
  <div>
    <span>ACTIVITY LOG</span>
    <strong>{showActivityLog ? "VISIBLE" : "HIDDEN"}</strong>
  </div>
  <button
    type="button"
    onClick={() => onShowActivityLogChange(!showActivityLog)}
    aria-pressed={showActivityLog}
    title={showActivityLog ? "Hide activity log" : "Show activity log"}
  >
    <PanelRight size={16} />
    {showActivityLog ? "HIDE" : "SHOW"}
  </button>
</div>
```

- [ ] **Step 5: Update app layout CSS**

In `ui/src/styles/app.css`, replace the base `.app-shell` grid rows with:

```css
.app-shell {
  display: grid;
  grid-template-rows: 58px minmax(0, 1fr);
  width: 100vw;
  height: 100vh;
  color: var(--color-text);
}

.app-shell.activity-log-visible {
  grid-template-rows: 58px minmax(0, 1fr) minmax(118px, 18vh);
}
```

Replace `.status-console` with:

```css
.status-console {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 6px;
  min-height: 0;
  padding: 6px;
  border-top: 1px solid var(--color-border);
  font-size: 11px;
}
```

In `@media (max-width: 1039px)`, replace the `.app-shell` rule with:

```css
.app-shell {
  height: 100vh;
  grid-template-rows: 58px minmax(0, 1fr);
}

.app-shell.activity-log-visible {
  grid-template-rows: 58px minmax(0, 1fr) minmax(118px, 22vh);
}
```

In `@media (max-width: 579px)`, replace the `.app-shell` rule with:

```css
.app-shell {
  grid-template-rows: 102px minmax(0, 1fr);
}

.app-shell.activity-log-visible {
  grid-template-rows: 102px minmax(0, 1fr) minmax(118px, 22vh);
}
```

- [ ] **Step 6: Run frontend tests and build**

Run:

```bash
pnpm test -- App.test.ts
pnpm build
```

Expected: PASS.

- [ ] **Step 7: Commit layout cleanup**

Run:

```bash
git add ui/src/App.tsx ui/src/App.test.ts ui/src/styles/app.css
git commit -m "feat: make activity log optional"
```

## Task 5: Browser Demo Direct Behavior

**Files:**
- Modify: `ui/src/data/demoBackend.ts`
- Modify: `ui/src/api.test.ts`
- Test: `ui/src/api.test.ts`

- [ ] **Step 1: Update demo API test**

In `ui/src/api.test.ts`, replace the queued-send demo test with this direct-send test.

```ts
it("sends directly in the browser demo and records Sent mail", async () => {
  const account = await api.saveAccountConfig({
    id: null,
    display_name: "Direct Send Demo",
    email: "direct-send-demo@example.com",
    password: "plain-mail-secret",
    imap_host: "imap.direct-send.example.com",
    imap_port: 993,
    imap_tls: true,
    smtp_host: "smtp.direct-send.example.com",
    smtp_port: 465,
    smtp_tls: true,
    sync_enabled: true
  });

  const sentMessageId = await api.sendMessage({
    account_id: account.id,
    to: ["sec@example.com"],
    cc: ["ops-lead@example.com"],
    subject: "Demo direct send",
    body: "Visible after SMTP success"
  });

  const sentFolder = (await api.listFolders(account.id)).find(
    (folder) => folder.account_id === account.id && folder.role === "sent"
  );
  expect(sentFolder).toBeDefined();
  expect(sentFolder?.total_count).toBe(1);
  expect(sentFolder?.unread_count).toBe(0);

  const sentMessages = await api.listMessages({
    account_id: account.id,
    folder_id: sentFolder?.id,
    limit: 10,
    offset: 0
  });
  const sentMessage = sentMessages.find((message) => message.id === sentMessageId);
  expect(sentMessage).toBeDefined();
  expect(sentMessage?.uid).toBeNull();
  expect(sentMessage?.message_id_header).toMatch(/^<.+@agentmail\.local>$/);
  expect(sentMessage?.sender).toBe(account.email);
  expect(sentMessage?.recipients).toEqual(["sec@example.com"]);
  expect(sentMessage?.cc).toEqual(["ops-lead@example.com"]);
  expect(sentMessage?.subject).toBe("Demo direct send");
  expect(sentMessage?.body).toBe("Visible after SMTP success");

  const audits = await api.getAuditLog(5);
  expect(audits[0].action).toBe("send");
  expect(audits[0].status).toBe("executed");
  expect(audits[0].message_ids).toEqual([sentMessageId]);
});
```

- [ ] **Step 2: Run failing demo API test**

Run:

```bash
pnpm test -- api.test.ts
```

Expected: FAIL because demo `send_message` still creates pending actions and records queued audit status.

- [ ] **Step 3: Remove pending state from demo backend**

In `ui/src/data/demoBackend.ts`, remove `PendingMailAction` from the type import and delete:

```ts
let pendingActions: PendingMailAction[] = [];
```

Keep `actionResult` because `MailActionResult` still supports the `pending_action_id` shape from the backend DTO, but direct paths should call `actionResult("executed")`.

- [ ] **Step 4: Make demo actions direct**

Replace the `execute_mail_action` case in `ui/src/data/demoBackend.ts` with:

```ts
case "execute_mail_action": {
  const request = args?.request as MailActionRequest;
  const trashFolder = folders.find((folder) => folder.account_id === request.account_id && folder.role === "trash");
  const archiveFolder = folders.find((folder) => folder.account_id === request.account_id && folder.role === "archive");
  const selected = messages.filter((message) => request.message_ids.includes(message.id));
  const isTrashDelete =
    request.action === "delete" &&
    selected.length > 0 &&
    selected.every((message) => folders.find((folder) => folder.id === message.folder_id)?.role === "trash");
  const recordedAction =
    isTrashDelete ? "permanent_delete" : request.action === "delete" && request.message_ids.length > 1 ? "batch_delete" : request.action;

  messages = messages.map((message) => {
    if (!request.message_ids.includes(message.id)) return message;
    if (request.action === "mark_read") return { ...message, flags: { ...message.flags, is_read: true } };
    if (request.action === "mark_unread") return { ...message, flags: { ...message.flags, is_read: false } };
    if (request.action === "star") return { ...message, flags: { ...message.flags, is_starred: true } };
    if (request.action === "unstar") return { ...message, flags: { ...message.flags, is_starred: false } };
    if (isTrashDelete) return { ...message, deleted_at: now() };
    if (request.action === "delete" && trashFolder) return { ...message, folder_id: trashFolder.id, uid: null };
    if ((request.action === "move" || request.action === "batch_move") && request.target_folder_id) {
      return { ...message, folder_id: request.target_folder_id, uid: null };
    }
    if (request.action === "archive" && archiveFolder) return { ...message, folder_id: archiveFolder.id, uid: null };
    return message;
  });
  folders = folders.map((folder) => ({
    ...folder,
    total_count: messages.filter((message) => message.folder_id === folder.id && !message.deleted_at).length,
    unread_count: messages.filter((message) => message.folder_id === folder.id && !message.deleted_at && !message.flags.is_read).length
  }));
  recordAudit(recordedAction, request.account_id, request.message_ids);
  return actionResult("executed");
}
```

- [ ] **Step 5: Make demo sends direct**

In the `send_message` case, remove the `pending` object and `pendingActions` write. Replace the final audit and return lines with:

```ts
recordAudit("send", draft.account_id, [messageId], "executed");
return messageId;
```

Remove the `list_pending_actions`, `confirm_action`, and `reject_action` cases from the demo backend switch.

- [ ] **Step 6: Update demo seed text**

In the seeded Sent message with id `msg-101`, replace the pending queue text with direct-send text.

```ts
body_preview: "Confirmation sent directly after compose submit.",
body: "Confirmation sent directly after compose submit.\n\nThis row exercises non-INBOX folder navigation in the browser demo.",
```

- [ ] **Step 7: Run demo tests**

Run:

```bash
pnpm test -- api.test.ts
```

Expected: PASS.

- [ ] **Step 8: Commit demo behavior**

Run:

```bash
git add ui/src/data/demoBackend.ts ui/src/api.test.ts
git commit -m "feat: align demo with direct send"
```

## Task 6: Documentation And Final Verification

**Files:**
- Modify: `README.md`
- Modify: `docs/PROJECT_STATUS.md`
- Modify: `docs/DECISIONS.md`
- Modify: `docs/NEXT_STEPS.md`
- Modify: `docs/REAL_MAIL_ACCEPTANCE.md`

- [ ] **Step 1: Update README current MVP bullets**

In `README.md`, replace the pending-action bullet with:

```markdown
- Mail actions and SMTP sends execute directly from the UI; action history is recorded in the audit log.
```

Replace the desktop UI shell bullet with:

```markdown
- Desktop UI shell with account/folder rail, message list, detail pane, compose modal, unified configuration modal, search, topbar sync controls, and an optional activity log footer.
```

- [ ] **Step 2: Update project status**

In `docs/PROJECT_STATUS.md`, replace the user-flow send line with:

```markdown
4. Compose mail; sending executes directly through SMTP and records an audit entry.
```

Replace the implemented SMTP bullet with:

```markdown
- SMTP send executes directly from the compose flow and records executed or failed audit entries.
```

Add this implemented UI bullet near the UI capabilities:

```markdown
- `AUDIT / ACTIVITY LOG` is hidden by default and can be shown from the `DISPLAY` settings tab.
```

Remove any current-limit or recent-status wording that says SMTP send must be confirmed in `PENDING ACTIONS`.

- [ ] **Step 3: Update decisions**

In `docs/DECISIONS.md`, replace the pending decision with:

```markdown
- SMTP send executes directly after compose submit; no extra pending-action confirmation is required.
```

Add this product decision:

```markdown
- Keep the activity log available, but hide it by default and expose it through display settings.
```

- [ ] **Step 4: Update next steps**

In `docs/NEXT_STEPS.md`, replace the compose/pending checklist lines with:

```markdown
- [ ] Compose a message and confirm it is sent directly.
- [ ] Verify the delivered message appears in Sent locally after successful send.
```

Replace the failed-send follow-up wording with:

```markdown
  - Show SMTP send failures clearly in main status text and the optional activity log.
```

- [ ] **Step 5: Update real mail acceptance**

In `docs/REAL_MAIL_ACCEPTANCE.md`, replace the sync acceptance line mentioning `SYNC & CONNECTIONS` with:

```markdown
- Manual sync can be triggered from the topbar sync button, and folder/message counts refresh after sync.
```

Replace send acceptance lines with:

```markdown
- Compose a test message; it should send directly without a `PENDING ACTIONS` confirmation step.
- Confirm the message is delivered and appears in Sent locally after successful send.
```

Add this display check:

```markdown
- In `CONFIGURATION` -> `DISPLAY`, enable `ACTIVITY LOG`; recent audit lines should appear in the footer. Disable it again; the footer should disappear and the mail workspace should reclaim the space.
```

- [ ] **Step 6: Run all verification commands**

Run:

```bash
cargo fmt --all --check
cargo test -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api
pnpm test
pnpm build
pnpm rust:check
git diff --check
```

Expected: PASS, with the known caveat that Linux Tauri GUI dependency checks can fail only if `pnpm rust:check` invokes platform pieces blocked by missing system packages.

- [ ] **Step 7: Browser verification**

Start the dev server:

```bash
pnpm dev -- --host 127.0.0.1
```

Verify in browser:

```text
1. Default desktop viewport opens with no footer and no reserved bottom console space.
2. Compose demo mail sends directly and closes the composer.
3. Sent folder shows the sent message.
4. Configuration -> DISPLAY -> ACTIVITY LOG -> SHOW displays the audit footer.
5. ACTIVITY LOG -> HIDE removes the footer and expands the workspace.
6. Narrow viewport keeps topbar, message list, detail pane, and optional footer from overlapping.
```

If Playwright MCP is blocked by a shared-browser lock, use `agent-browser` for the same checks.

- [ ] **Step 8: Commit documentation and verification updates**

Run:

```bash
git add README.md docs/PROJECT_STATUS.md docs/DECISIONS.md docs/NEXT_STEPS.md docs/REAL_MAIL_ACCEPTANCE.md
git commit -m "docs: update direct send acceptance"
```

## Self-Review

- Spec coverage: direct send, direct normalized actions, compatibility retention for `pending_actions`, removal of pending frontend state, removal of `SYNC & CONNECTIONS`, settings-controlled `AUDIT / ACTIVITY LOG`, demo behavior, docs, and verification are each covered by a task.
- Type consistency: `formatSendStatus`, `ACTIVITY_LOG_STORAGE_KEY`, `readStoredActivityLogVisibility`, `writeStoredActivityLogVisibility`, and `getAppShellClassName` are introduced before they are used in UI tests and component code.
- Data consistency: successful direct sends persist a Sent message and executed audit transactionally through `save_direct_sent_message`; failed SMTP sends write failed audit and do not write a Sent message.
