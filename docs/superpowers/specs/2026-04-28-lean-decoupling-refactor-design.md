# Lean Decoupling Refactor Design

## Goal

Reduce redundancy and improve module boundaries without changing AgentMail's current product behavior: direct mail actions, direct SMTP send, optional activity log, folder sync, and manual remote AI analysis must continue to behave as they do on `main`.

## Current Findings

- `crates/app-api/src/lib.rs` is the main backend hot spot at roughly 4k lines. It mixes account configuration, sync orchestration, mail actions, SMTP send, AI analysis, pending-action compatibility, DTOs, helper functions, and tests.
- `crates/mail-store/src/lib.rs` is the persistence hot spot at roughly 2.4k lines. It mixes migration SQL, repository methods, transaction helpers, row mapping, enum string codecs, and tests.
- `ui/src/App.tsx` is the frontend hot spot at roughly 1.7k lines. It mixes app state orchestration, sync side effects, workspace layout, settings modal, composer, AI panel, storage helpers, and formatting helpers.
- Rust crate dependencies are directionally healthy: `mail-core` is the shared DTO base; `mail-store`, `mail-protocol`, and `ai-remote` depend inward on it; `app-api` orchestrates those crates; Tauri depends on `app-api` and `mail-core`.
- Product pending-action UI has been removed, but compatibility code remains in `mail-core`, `mail-store`, `app-api`, and Tauri. The SQLite table should remain for now, but the compatibility surface should be isolated and not leak into active product paths.
- `cargo clippy --all-targets -- -D warnings` currently fails on small redundancy warnings, which makes it a useful first cleanup gate.

## Non-Goals

- Do not drop the `pending_actions` SQLite table or migration code in this pass.
- Do not replace Tauri, React, Vite, SQLite, or the Rust workspace layout.
- Do not rewrite the UI visually.
- Do not change mail sync semantics, direct send semantics, AI settings storage, or real-mail acceptance behavior.
- Do not introduce a new state-management framework unless later evidence shows the React split alone is insufficient.

## Recommended Approach

Use a staged refactor. Each stage must keep tests green and should reduce one kind of coupling at a time.

### Stage 1: Low-Risk Redundancy Cleanup

Fix compiler/linter-visible redundancy and remove obviously unused helpers that no product path depends on.

- Derive `Default` for `AiPriority` instead of manually implementing it.
- Simplify clippy-reported redundant expressions in store/protocol/app-api tests and helpers.
- Remove or quarantine `MailActionKind::requires_confirmation` only after confirming no remaining code path calls it.
- Keep pending-action domain types if store/app-api compatibility still needs them.

### Stage 2: Frontend Helper Extraction

Split pure and orchestration helpers out of `App.tsx` before moving React components.

Target modules:

- `ui/src/lib/storage.ts`: theme, activity-log, and workspace split localStorage helpers.
- `ui/src/lib/format.ts`: time, size, folder count, audit line, send status.
- `ui/src/lib/syncFlows.ts`: `runInitialAccountSync`, `refreshAfterMailSyncEvent`, `runAutomaticAccountSync`, `runManualAccountSync`.
- `ui/src/lib/mailActions.ts`: action labels and lightweight action helpers.

This keeps tests focused and makes the main React component smaller without changing rendered markup.

### Stage 3: Frontend Component Extraction

Move leaf components after their helpers are already extracted.

Target modules:

- `ui/src/components/AiPanel.tsx`
- `ui/src/components/Composer.tsx`
- `ui/src/components/ConfigurationModal.tsx`

These components can remain controlled by props. Avoid adding global state or context unless the props become unmanageable after extraction.

### Stage 4: Backend Boundary Extraction

Split `app-api` into responsibility modules while preserving the public `AppApi` API.

Target modules:

- `crates/app-api/src/accounts.rs`: account config DTOs and account connection helpers.
- `crates/app-api/src/sync.rs`: sync/backoff/watch helpers and sync summary.
- `crates/app-api/src/mail_actions.rs`: direct action normalization, remote action building, local application, and action audit writing.
- `crates/app-api/src/send.rs`: direct SMTP send, Sent placeholder creation/reconciliation, address normalization.
- `crates/app-api/src/ai.rs`: AI settings view/mask, analysis input, provider orchestration.
- `crates/app-api/src/pending_compat.rs`: `list_pending_actions`, `confirm_action`, `reject_action`, and queued-send placeholder cleanup.

The first backend pass should move code without changing semantics. Only after modules are stable should we delete or shrink compatibility code.

### Stage 5: Store Boundary Extraction

Split `mail-store` after `app-api` is less coupled to it.

Target modules:

- `crates/mail-store/src/schema.rs`: migration SQL and migration helpers.
- `crates/mail-store/src/rows.rs`: row-to-domain mappers.
- `crates/mail-store/src/codecs.rs`: enum/string conversions.
- `crates/mail-store/src/messages.rs`, `accounts.rs`, `sync_state.rs`, `actions.rs`, `ai.rs`: repository method groups.

This is a larger move-only refactor and should be done after the frontend and app-api changes have reduced active churn.

## Design Constraints

- Prefer move-only refactors first. Behavior changes require focused tests.
- Keep compatibility code explicit. If pending-action functions remain, their module name must make it clear they are legacy compatibility, not current product behavior.
- Keep `README.md` as the root entry point and project docs under `docs/`.
- Do not edit historical `docs/superpowers/*` except for this Superpowers-generated design and its implementation plan.
- Use `pnpm`, not `npm`.

## Verification

Each implementation batch must run the narrow tests for touched code plus the relevant full gates:

```bash
cargo fmt --all --check
cargo clippy -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api --all-targets -- -D warnings
cargo test -p mail-core -p mail-store -p mail-protocol -p ai-remote -p app-api
pnpm test
pnpm build
pnpm rust:check
git diff --check
```

Tauri desktop compile checks may still be blocked on this Linux environment by missing WebKit/GTK system packages. Do not treat that blocker as app-code evidence unless a Windows or properly provisioned Linux builder also fails.

## Rollout Strategy

1. Commit this design and a task-level implementation plan.
2. Execute Stage 1 first because it is low risk and creates a cleaner lint baseline.
3. Execute Stage 2 before Stage 3 so component extraction does not move logic and markup at the same time.
4. Execute Stage 4 in small backend slices with no public API changes.
5. Defer Stage 5 unless the earlier stages pass cleanly and the store file remains a practical bottleneck.
