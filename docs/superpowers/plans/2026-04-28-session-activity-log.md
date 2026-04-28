# Session Activity Log Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:test-driven-development for helper behavior and superpowers:verification-before-completion before committing. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the persisted audit-row footer with a per-session diagnostic text log that starts empty on app launch, appends key behavior events, and scrolls as a text area.

**Architecture:** Keep backend audit storage unchanged. Add a small frontend helper for log entries and text formatting, wire `App.tsx` to append diagnostic lines around key async flows, and render the existing `AUDIT / ACTIVITY LOG` footer as one read-only textarea-style log.

**Tech Stack:** React/Vite TypeScript UI, Vitest, existing `ACTIVITY LOG` display setting.

---

## File Structure

- Create `ui/src/lib/activityLog.ts`: pure helpers for appending session log entries and building textarea text.
- Modify `ui/src/App.test.ts`: add helper tests.
- Modify `ui/src/App.tsx`: add session log state, append key behavior lines, and replace per-audit rows with a readonly text box.
- Modify `ui/src/styles/app.css`: make the activity log text box fill the footer and scroll.

## Task 1: Add Tested Activity Log Helpers

- [ ] Add failing tests in `ui/src/App.test.ts`:

```ts
import { appendActivityLogEntry, buildActivityLogText } from "./lib/activityLog";

describe("activity log helpers", () => {
  it("appends timestamped session log entries without mutating previous entries", () => {
    const existing = appendActivityLogEntry([], "app startup", () => new Date("2026-04-28T09:00:00Z"));
    const next = appendActivityLogEntry(existing, "manual sync started", () => new Date("2026-04-28T09:00:05Z"));

    expect(existing).toHaveLength(1);
    expect(next).toEqual([
      { id: 1, timestamp: "2026-04-28T09:00:00.000Z", message: "app startup" },
      { id: 2, timestamp: "2026-04-28T09:00:05.000Z", message: "manual sync started" }
    ]);
  });

  it("formats session log entries as one newline-delimited text block", () => {
    const text = buildActivityLogText([
      { id: 1, timestamp: "2026-04-28T09:00:00.000Z", message: "app startup" },
      { id: 2, timestamp: "2026-04-28T09:00:05.000Z", message: "manual sync started" }
    ]);

    expect(text).toContain("app startup");
    expect(text).toContain("\n");
    expect(text).toContain("manual sync started");
  });
});
```

- [ ] Run `pnpm test App.test.ts`; expected failure because `./lib/activityLog` does not exist.
- [ ] Create `ui/src/lib/activityLog.ts` with `ActivityLogEntry`, `appendActivityLogEntry`, and `buildActivityLogText`.
- [ ] Run `pnpm test App.test.ts`; expected pass.

## Task 2: Wire App Behavior Logging And Text Box UI

- [ ] In `ui/src/App.tsx`, add `activityLogEntries` state initialized to `[]`, append `"app startup"` in mount effect, and define an `appendActivityLog(message)` callback.
- [ ] Append key diagnostic lines for startup refresh, selected account/folder refresh failures, mail sync events, manual sync start/finish/failure, account save/initial sync, send flow, mail actions, and AI analysis.
- [ ] Replace the footer audit list with a single readonly textbox:

```tsx
<textarea className="activity-log-textbox" value={buildActivityLogText(activityLogEntries)} readOnly aria-label="Activity log history" />
```

- [ ] In `ui/src/styles/app.css`, remove per-row audit card styling and make `.activity-log-textbox` fill the panel with vertical scrolling.
- [ ] Run `pnpm test App.test.ts` and `pnpm build`; expected pass.

## Task 3: Verify And Commit

- [ ] Run `pnpm test`.
- [ ] Run `pnpm build`.
- [ ] Run `pnpm rust:check`.
- [ ] Run `git diff --check`.
- [ ] Commit with `feat: add session activity log`.
