# Message Reading Layout Design

Date: 2026-05-12

## Goal

Make the message detail pane behave like a mail reader: the action toolbar, subject, sender and recipient metadata stay fixed; only the message body scrolls; the AI panel remains fixed at the bottom of the detail pane.

## Current Context

The selected-message UI lives in `ui/src/App.tsx`. Today the detail pane renders:

- `.detail-toolbar`
- `.detail-scroll`
  - `.message-detail`
    - `.message-heading`
    - `.message-envelope`
    - body block or HTML iframe
    - attachment strip
  - `AiPanel`

`ui/src/styles/app.css` makes `.detail-scroll` the scroll container. That means the message heading, mail header card, body, attachments, and AI panel share one scroll surface. The current `.message-envelope` uses `position: sticky`, but that only pins part of the metadata inside the same scroll container and does not match the desired three-zone reader layout.

The user-provided screenshot marks the intended layout:

- Top fixed zone: toolbar, subject, sender/recipient metadata.
- Middle scroll zone: message body only.
- Bottom fixed zone: AI status, controls, and results.

## Chosen Approach

Use a structural layout split instead of adding more sticky positioning.

`detail-pane` should remain the root detail grid and become:

1. `.detail-toolbar` for the existing Star/Delete action row.
2. `.message-reading-shell` for the selected message.
3. `.ai-panel` for AI status and analysis.

`message-reading-shell` should be a two-row grid:

1. `.message-header-fixed`, containing the existing `.message-heading` and `.message-envelope`.
2. `.message-body-scroll`, containing the existing HTML/plaintext body and attachment strip.

The only vertical scroll inside an opened message should be `.message-body-scroll`. The AI panel should be a sibling of `message-reading-shell`, not inside a scroll container.

## UI Requirements

- Keep the current visual language, colors, borders, and dense layout.
- Keep the current toolbar actions and disabled states unchanged.
- Keep the current mail header content unchanged: subject, read/unread dot, sender identity, recipients, CC, time, size, UID, and Message-ID.
- Keep the existing HTML rendering path, iframe sandbox, plaintext fallback, and attachment chips unchanged.
- Remove sticky behavior from `.message-envelope`; fixed positioning should come from grid rows.
- Use `min-height: 0` on every grid/flex boundary that contains a scroll area, so the body scroll surface can shrink correctly.
- Preserve modal layering: configuration/composer modals must remain above the message header.
- Preserve the resizable list/detail split and narrow-screen stacked layout.
- On narrow screens, the detail pane should still fit inside the available viewport; if space is tight, the body scroll area shrinks before the header or AI panel overlap.

## Non-Goals

- No backend or API changes.
- No MIME parsing, sanitizer, iframe, or remote image behavior changes.
- No redesign of the AI panel content.
- No new user-visible controls.
- No changes to message list behavior, account rail behavior, sync, send, delete, or search.

## Testing

Frontend unit/layout tests should cover the contract:

- Rendering an HTML message still creates `.body-html-block` and `.body-html-frame`.
- The selected-message DOM has a fixed header container, a body-only scroll container, and an AI panel outside the scroll container.
- The fixed header container includes `.message-heading` and `.message-envelope`.
- The body scroll container includes the body block or HTML body and attachments.
- The AI panel is a sibling of the reading shell, not a child of `.message-body-scroll`.
- Existing message envelope border and modal z-index tests remain valid.
- The medium-width responsive helper is updated to represent body-scroll height rather than old whole-detail scroll height.

Verification should include:

```bash
pnpm test -- ui/src/App.test.ts
pnpm test
pnpm build
git diff --check
```

For visual confidence, run the app and inspect at least one desktop viewport with a selected HTML message. The visible result should match the screenshot intent: the top mail identity block and bottom AI panel remain in place while the body area scrolls.

## Acceptance Criteria

- In message browsing, the only scrollable area inside the detail pane is the message body/attachment region.
- Toolbar, subject, sender/recipient metadata, and secondary metadata remain fixed while reading a long message.
- The AI panel remains fixed below the message body area and does not scroll with the message body.
- Existing HTML and plaintext rendering behavior is preserved.
- Existing message header border and modal layering regressions remain covered by tests.
