# Message Reading Layout Plan

Date: 2026-05-12

## Goal

Keep message browsing focused on the mail body: the detail toolbar, subject,
sender/recipient metadata, and AI panel stay fixed while only the message body
and attachments scroll.

## Scope

- Change the selected-message detail layout in `ui/src/App.tsx`.
- Adjust the detail-pane layout CSS in `ui/src/styles/app.css`.
- Cover the structural behavior in `ui/src/App.test.ts`.
- Leave backend mail behavior, MIME rendering, iframe sandboxing, sync, send,
  delete, and AI invocation behavior unchanged.

## Design Decision

Use a structural grid split instead of more sticky positioning.

The detail pane is arranged as:

1. `.detail-toolbar`
2. `.message-reading-shell`
3. `.ai-panel`

The message shell is arranged as:

1. `.message-header-fixed`
2. `.message-body-scroll`

This keeps the AI panel outside the scroll surface and makes
`.message-body-scroll` the only body-reading scroll region.

## Implementation Tasks

- [x] Move `.message-heading` and `.message-envelope` into
  `.message-header-fixed`.
- [x] Move HTML/plaintext body rendering and attachments into
  `.message-body-scroll`.
- [x] Render `AiPanel` as a sibling of `.message-reading-shell`, not a child of
  a scroll container.
- [x] Replace the old whole-detail scroll CSS with a three-row detail grid and
  a two-row message shell.
- [x] Preserve responsive split behavior and narrow-screen stacking.
- [x] Update responsive layout helper expectations to describe body-scroll
  height instead of old detail-scroll height.
- [x] Add a DOM test proving metadata and AI stay outside the body-only scroll
  region.

## Review Follow-Up

- [x] Remove the exported message-reading contract helper because the DOM test
  already covers the real structural relationship and the helper only exposed
  CSS class details from the production module.
- [x] Compress this plan into an implementation record; keep detailed rationale
  in the design spec.

## Verification

Commands to run after changes:

```bash
pnpm test -- ui/src/App.test.ts
pnpm test
pnpm build
git diff --check
```

Browser verification:

- Select an HTML message.
- Confirm `.message-reading-shell` contains `.message-header-fixed` and
  `.message-body-scroll`.
- Confirm `.ai-panel` is outside `.message-reading-shell` and outside
  `.message-body-scroll`.
- Confirm `.message-body-scroll` has `overflow-y: auto`.

## Deferred

- Reading space can still be improved for small viewports or messages with long
  recipient metadata. Possible follow-up: reduce AI panel height, cap/collapse
  the header block, or make the AI panel collapsible.
