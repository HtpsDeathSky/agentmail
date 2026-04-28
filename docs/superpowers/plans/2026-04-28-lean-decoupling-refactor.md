# Lean Decoupling Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce AgentMail redundancy and coupling through low-risk cleanup plus frontend helper extraction while preserving current product behavior.

**Architecture:** First make lint-visible redundancy fail/pass cleanly, then move pure frontend helpers out of `App.tsx` before moving any React components. Keep Rust crate boundaries intact in this first pass and do not remove the `pending_actions` database table or compatibility repository methods.

**Tech Stack:** Rust workspace (`mail-core`, `mail-store`, `mail-protocol`, `ai-remote`, `app-api`), React/Vite TypeScript UI, Vitest, Cargo, pnpm.

---

## File Structure

- Modify `crates/mail-core/src/lib.rs`: derive `Default` for `AiPriority`; remove unused `MailActionKind::requires_confirmation` after verifying no call sites.
- Modify `crates/mail-store/src/lib.rs`: simplify clippy-reported redundant test expression only.
- Modify `crates/mail-protocol/src/lib.rs`: simplify clippy-reported consecutive `replace` call.
- Modify `crates/app-api/src/lib.rs`: simplify clippy-reported nested `if` and needless struct updates in tests.
- Create `ui/src/lib/storage.ts`: localStorage helpers for theme, activity log, and workspace split.
- Create `ui/src/lib/format.ts`: formatting helpers and action labels.
- Create `ui/src/lib/syncFlows.ts`: direct send and account sync orchestration helpers.
- Modify `ui/src/App.tsx`: import extracted helpers and remove their inline definitions.
- Modify `ui/src/App.test.ts`: import helper tests from new modules instead of `App.tsx`.

## Task 1: Rust Low-Risk Redundancy Cleanup

**Files:**
- Modify: `crates/mail-core/src/lib.rs`
- Modify: `crates/mail-store/src/lib.rs`
- Modify: `crates/mail-protocol/src/lib.rs`
- Modify: `crates/app-api/src/lib.rs`

- [ ] **Step 1: Verify the clippy red baseline**

Run:

```bash
cargo clippy -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api --all-targets -- -D warnings
```

Expected: FAIL with warnings promoted to errors, including:

- `clippy::derivable_impls` for `AiPriority`
- `clippy::let_and_return` in `mail-store` tests
- `clippy::collapsible_str_replace` in `mail-protocol`
- `clippy::collapsible_if` in `app-api`
- `clippy::needless_update` in `app-api` tests

- [ ] **Step 2: Confirm `requires_confirmation` has no call sites**

Run:

```bash
rg -n "requires_confirmation" crates src-tauri ui/src
```

Expected: only the method definition in `crates/mail-core/src/lib.rs`.

- [ ] **Step 3: Apply the Rust cleanup**

In `crates/mail-core/src/lib.rs`, replace the `AiPriority` enum and manual default impl with:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiPriority {
    Low,
    #[default]
    Normal,
    High,
    Urgent,
}
```

Also remove this unused impl block:

```rust
impl MailActionKind {
    pub fn requires_confirmation(self) -> bool {
        matches!(
            self,
            Self::PermanentDelete
                | Self::Send
                | Self::Forward
                | Self::BatchDelete
                | Self::BatchMove
        )
    }
}
```

In `crates/mail-protocol/src/lib.rs`, replace:

```rust
value.replace('\r', " ").replace('\n', " ")
```

with:

```rust
value.replace(['\r', '\n'], " ")
```

In `crates/app-api/src/lib.rs`, collapse the nested successful-send cleanup condition to:

```rust
if account.sync_enabled
    && self
        .reconcile_sent_placeholders_after_send(&account.id)
        .await
        .is_ok()
{
    if let Err(err) = self.cleanup_direct_sent_copy_after_reconcile(&account.id, &message_id) {
        warning = Some(format!("sent but local cleanup failed: {err}"));
    }
}
```

In the two `WatchProtocol` test initializers, remove `..WatchProtocol::default()` when all struct fields are already specified.

In `crates/mail-store/src/lib.rs`, replace the clippy-reported `let has_cascade_message_fk = ...; has_cascade_message_fk` test block with the direct expression returned from `.any(...)`.

- [ ] **Step 4: Verify Rust cleanup**

Run:

```bash
cargo fmt --all --check
cargo clippy -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api --all-targets -- -D warnings
cargo test -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api
```

Expected: all commands PASS.

- [ ] **Step 5: Commit Rust cleanup**

Run:

```bash
git add crates/mail-core/src/lib.rs crates/mail-store/src/lib.rs crates/mail-protocol/src/lib.rs crates/app-api/src/lib.rs
git commit -m "refactor: clean rust redundancy warnings"
```

## Task 2: Extract Frontend Storage Helpers

**Files:**
- Create: `ui/src/lib/storage.ts`
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/App.test.ts`

- [ ] **Step 1: Move storage constants and helpers**

Create `ui/src/lib/storage.ts` with:

```ts
export const THEME_MODE_STORAGE_KEY = "agentmail-theme-mode";
export const WORKSPACE_SPLIT_STORAGE_KEY = "agentmail-workspace-split-percent";
export const ACTIVITY_LOG_STORAGE_KEY = "agentmail-show-activity-log";
export const DEFAULT_WORKSPACE_SPLIT_PERCENT = 45;

export type ThemeMode = "dark" | "light";

export function readStoredThemeMode(storage: Pick<Storage, "getItem"> | null | undefined): ThemeMode {
  try {
    return storage?.getItem(THEME_MODE_STORAGE_KEY) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

export function getNextThemeMode(mode: ThemeMode): ThemeMode {
  return mode === "dark" ? "light" : "dark";
}

export function readStoredActivityLogVisibility(storage: Pick<Storage, "getItem"> | null | undefined) {
  try {
    return storage?.getItem(ACTIVITY_LOG_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

export function writeStoredActivityLogVisibility(storage: Pick<Storage, "setItem"> | null | undefined, visible: boolean) {
  try {
    storage?.setItem(ACTIVITY_LOG_STORAGE_KEY, visible ? "true" : "false");
  } catch {
    // Storage can be unavailable in hardened desktop/webview contexts.
  }
}

export function applyThemeModeToDocument(
  root: Pick<HTMLElement, "dataset" | "style">,
  storage: Pick<Storage, "setItem"> | null | undefined,
  mode: ThemeMode
) {
  root.dataset.theme = mode;
  root.style.colorScheme = mode;
  try {
    storage?.setItem(THEME_MODE_STORAGE_KEY, mode);
  } catch {
    // Storage can be unavailable in hardened desktop/webview contexts.
  }
}

export function readStoredWorkspaceSplitPercent(storage: Pick<Storage, "getItem"> | null | undefined) {
  try {
    const stored = storage?.getItem(WORKSPACE_SPLIT_STORAGE_KEY);
    if (!stored) return DEFAULT_WORKSPACE_SPLIT_PERCENT;
    const percent = Number(stored);
    return Number.isFinite(percent) && percent > 0 && percent < 100 ? percent : DEFAULT_WORKSPACE_SPLIT_PERCENT;
  } catch {
    return DEFAULT_WORKSPACE_SPLIT_PERCENT;
  }
}

export function writeStoredWorkspaceSplitPercent(storage: Pick<Storage, "setItem"> | null | undefined, percent: number) {
  try {
    storage?.setItem(WORKSPACE_SPLIT_STORAGE_KEY, String(percent));
  } catch {
    // Storage can be unavailable in hardened desktop/webview contexts.
  }
}
```

- [ ] **Step 2: Update imports and remove inline storage helpers**

In `ui/src/App.tsx`, import the storage symbols from `./lib/storage` and remove the inline definitions for:

- `ThemeMode`
- `THEME_MODE_STORAGE_KEY`
- `WORKSPACE_SPLIT_STORAGE_KEY`
- `ACTIVITY_LOG_STORAGE_KEY`
- `DEFAULT_WORKSPACE_SPLIT_PERCENT`
- `readStoredThemeMode`
- `getNextThemeMode`
- `readStoredActivityLogVisibility`
- `writeStoredActivityLogVisibility`
- `applyThemeModeToDocument`
- `readStoredWorkspaceSplitPercent`
- `writeStoredWorkspaceSplitPercent`

In `ui/src/App.test.ts`, import storage helpers from `./lib/storage`.

- [ ] **Step 3: Verify storage extraction**

Run:

```bash
pnpm test App.test.ts
pnpm build
```

Expected: both commands PASS.

- [ ] **Step 4: Commit storage extraction**

Run:

```bash
git add ui/src/lib/storage.ts ui/src/App.tsx ui/src/App.test.ts
git commit -m "refactor: extract frontend storage helpers"
```

## Task 3: Extract Frontend Format And Action Helpers

**Files:**
- Create: `ui/src/lib/format.ts`
- Create: `ui/src/lib/mailActions.ts`
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/App.test.ts`

- [ ] **Step 1: Create action labels module**

Create `ui/src/lib/mailActions.ts` with:

```ts
import { MailActionKind } from "../api";

export const actionLabels: Record<MailActionKind, string> = {
  mark_read: "READ",
  mark_unread: "UNREAD",
  star: "STAR",
  unstar: "UNSTAR",
  move: "MOVE",
  archive: "ARCHIVE",
  delete: "DELETE",
  permanent_delete: "PURGE",
  send: "SEND",
  forward: "FORWARD",
  batch_delete: "BATCH DELETE",
  batch_move: "BATCH MOVE"
};
```

- [ ] **Step 2: Create format module**

Create `ui/src/lib/format.ts` with:

```ts
import { MailActionAudit, MailFolder } from "../api";
import { actionLabels } from "./mailActions";

export function formatFolderCount(folder: Pick<MailFolder, "unread_count" | "total_count">) {
  if (folder.unread_count > 0) return `${folder.unread_count}/${folder.total_count}`;
  return String(folder.total_count);
}

export function formatSendStatus(recipients: string[]) {
  return `sent to ${recipients.join(", ")}`;
}

export function formatAuditLine(audit: MailActionAudit) {
  const base = `[${formatTime(audit.created_at)}] ${actionLabels[audit.action] ?? audit.action}:${audit.status}`;
  return audit.error_message ? `${base} / ${audit.error_message}` : base;
}

export function formatTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "unknown";
  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(date);
}

export function formatSize(value?: number | null) {
  if (!value) return "0 B";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${Math.round(value / 1024)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}
```

- [ ] **Step 3: Update imports and remove inline format helpers**

In `ui/src/App.tsx`, import:

```ts
import { actionLabels } from "./lib/mailActions";
import { formatAuditLine, formatFolderCount, formatSendStatus, formatSize, formatTime } from "./lib/format";
```

Remove inline `actionLabels`, `formatFolderCount`, `formatSendStatus`, `formatAuditLine`, `formatTime`, and `formatSize`.

In `ui/src/App.test.ts`, import `formatAuditLine` from `./lib/format` if tests still cover it.

- [ ] **Step 4: Verify format extraction**

Run:

```bash
pnpm test App.test.ts
pnpm build
```

Expected: both commands PASS.

- [ ] **Step 5: Commit format extraction**

Run:

```bash
git add ui/src/lib/format.ts ui/src/lib/mailActions.ts ui/src/App.tsx ui/src/App.test.ts
git commit -m "refactor: extract frontend format helpers"
```

## Task 4: Extract Frontend Sync Flow Helpers

**Files:**
- Create: `ui/src/lib/syncFlows.ts`
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/App.test.ts`

- [ ] **Step 1: Move flow request types and helper functions**

Create `ui/src/lib/syncFlows.ts` and move these exports from `App.tsx` into it:

- `MailSyncEventPayload`
- `DirectSendFlowRequest`
- `DirectSendFlowResult`
- `runDirectSendFlow`
- `InitialAccountSyncRequest`
- `runInitialAccountSync`
- `RefreshAfterMailSyncEventRequest`
- `refreshAfterMailSyncEvent`
- `AutomaticAccountSyncRequest`
- `runAutomaticAccountSync`
- `ManualAccountSyncRequest`
- `runManualAccountSync`

Keep `firstRejectedReason` private inside `syncFlows.ts`.

The module should import only:

```ts
import { SendMessageDraft, SendMessageResult, SyncSummary } from "../api";
import { formatSendStatus } from "./format";
```

Define `MailSyncEventPayload` in `syncFlows.ts`. `App.tsx` must import that type from `./lib/syncFlows`; `syncFlows.ts` must not import from `App.tsx`.

- [ ] **Step 2: Update App imports and remove inline flow helpers**

In `ui/src/App.tsx`, import the flow helpers from `./lib/syncFlows` and remove the inline type/function definitions.

In `ui/src/App.test.ts`, import the flow helpers from `./lib/syncFlows`.

- [ ] **Step 3: Verify sync flow extraction**

Run:

```bash
pnpm test App.test.ts
pnpm build
```

Expected: both commands PASS.

- [ ] **Step 4: Commit sync flow extraction**

Run:

```bash
git add ui/src/lib/syncFlows.ts ui/src/App.tsx ui/src/App.test.ts
git commit -m "refactor: extract frontend sync flows"
```

## Task 5: Full Verification And Review

**Files:**
- Verification and review only. Do not edit source files in this task unless a verification command exposes a concrete regression; any fix must be followed by re-running the failed verification command.

- [ ] **Step 1: Run full verification**

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

Expected: all commands PASS.

- [ ] **Step 2: Inspect final diff scope**

Run:

```bash
git status --short --branch
git log --oneline --decorate -8
```

Expected: worktree is clean after commits, and the latest commits are the refactor commits from this plan.

- [ ] **Step 3: Final code review**

Review the full range from the design commit parent through `HEAD` for:

- Behavior-preserving refactor only.
- No new pending-action product UI or frontend API.
- No new dependency cycle.
- `App.tsx` smaller and helper modules focused.
- Clippy clean with warnings denied.

Expected: no Critical or Important issues remain.
