# Mail List Actions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make mail list selection and context-menu actions operate on the intended message, including unopened messages.

**Architecture:** Keep backend APIs unchanged because `execute_mail_action` and `run_ai_analysis` already accept message IDs. Add small frontend helpers for target-aware actions, wire `App.tsx` to use explicit message targets instead of only `selectedMessageId`, and keep the context menu as a thin UI layer.

**Tech Stack:** React 18, TypeScript, Vitest, Tauri command API.

---

### Task 1: Action Helper Tests

**Files:**
- Modify: `ui/src/lib/mailActions.ts`
- Modify: `ui/src/App.test.ts`

- [ ] Add tests for `shouldAutoMarkRead`, `getContextMenuActionItems`, and target-aware AI refresh behavior.
- [ ] Run `pnpm test -- ui/src/App.test.ts` and verify the new tests fail because helpers do not exist.
- [ ] Implement the helpers in `ui/src/lib/mailActions.ts`.
- [ ] Run `pnpm test -- ui/src/App.test.ts` and verify the helper tests pass.

### Task 2: App Wiring

**Files:**
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/styles/app.css`

- [ ] Replace selected-message-only action handling with `runMessageAction(message, action, targetFolderId?)`.
- [ ] Auto-mark unread messages as read after they are selected and fetched.
- [ ] Remove the manual READ/UNREAD button from the detail toolbar.
- [ ] Add a right-click menu on `.message-row` with READ, STAR/UNSTAR, DELETE, and ANALYZE.
- [ ] Make context-menu ANALYZE run against the target message without selecting it; refresh the AI panel only when the target is already selected.
- [ ] Add dark/light theme CSS for the menu.

### Task 3: Verification

**Files:**
- Verify only.

- [ ] Run `pnpm test`.
- [ ] Run `pnpm build`.
- [ ] Run `cargo fmt --all --check`.
- [ ] Run `cargo test -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api`.
- [ ] Run `git diff --check`.
