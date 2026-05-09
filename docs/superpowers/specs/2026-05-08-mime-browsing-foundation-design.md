# MIME Browsing Foundation Design

Date: 2026-05-08

## Goal

Improve AgentMail's message browsing by preserving raw MIME during sync, rendering richer HTML mail safely, loading remote images by default, displaying inline CID images, and making the message header area read like a mail client instead of database fields.

## First-Phase Scope

This phase is a browsing foundation, not a full attachment release.

Included:

- Save the original RFC822 raw MIME source for every synced remote message.
- Preserve raw MIME across normal message upserts.
- Delete stored raw MIME whenever the local message is permanently removed, including Trash-folder permanent delete cleanup.
- Parse and expose HTML body content separately from plain text body content.
- Render sanitized HTML in the message detail pane when available.
- Load remote images by default.
- Support inline `cid:` images used by `multipart/related` HTML messages.
- Improve the visible `From`, `To`, `Cc`, time, size, UID, and Message-ID presentation in the detail pane.

Excluded:

- Ordinary attachment download, open, save-as, or reveal-in-folder behavior.
- PDF, Office, or image attachment preview outside inline CID images.
- MIME structure tree or professional debugging view.
- `.eml` import/export.
- Remote-image privacy blocking or sender trust lists.
- S/MIME, PGP, calendar invitations, rules, templates, contacts, and tray behavior.

## Current Context

`crates/mail-protocol/src/lib.rs` already fetches complete messages with `BODY.PEEK[]` and parses them through `mail_parser`. The current `MailMessage` model in `crates/mail-core/src/lib.rs` only carries `body`, `body_preview`, and attachment metadata. HTML-only messages are converted to plain text with a simple local `html_to_text` helper, so the UI loses the original HTML presentation.

`crates/mail-store/src/lib.rs` stores messages in SQLite and uses FTS over subject, sender, recipients, and text body. Attachment rows exist but current product docs state that files are not downloaded. `ui/src/App.tsx` renders the selected body in a `<pre className="body-block">` and shows attachment metadata as inert chips.

## Architecture

### Raw MIME Storage

Add a dedicated raw-source table instead of a large nullable column on `messages`:

```sql
CREATE TABLE IF NOT EXISTS message_raw_sources (
  message_id TEXT PRIMARY KEY,
  raw_mime BLOB NOT NULL,
  stored_at TEXT NOT NULL,
  FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE
);
```

The protocol layer should set `raw_mime` on synced `MailMessage` values from the exact bytes returned by IMAP `BODY.PEEK[]`. The store upsert path writes or replaces the raw-source row in the same transaction as the message row. Local placeholder messages and sent placeholders can keep `raw_mime = None`.

Permanent deletion must remove raw MIME. The foreign key cascade handles ordinary message-row deletes. Any manual cleanup path that deletes from `messages` must either rely on cascade with foreign keys enabled or explicitly delete from `message_raw_sources` before deleting the message. This requirement specifically covers Trash-folder permanent delete.

### Parsed Message Body

Extend the shared message model with MIME browsing fields:

- `body`: existing plain text body, retained for AI input and FTS.
- `html_body`: sanitized or unsanitized original HTML string for display preparation. The implementation may store the original parsed HTML and sanitize in the frontend, but the UI must only inject sanitized HTML.
- `raw_mime`: optional raw bytes, kept out of the normal frontend `MailMessage` payload unless an API specifically needs it.
- `inline_parts`: metadata and data URLs for inline CID resources needed by HTML body rendering.

The first implementation can use base64 `data:` URLs for inline CID resources to avoid adding a Tauri asset protocol. The CID replacement map should normalize `Content-ID` values with and without angle brackets so `cid:logo@example` and `<logo@example>` match.

### HTML Safety

Remote images are allowed by default. Security protection still applies:

- Remove `script`, `iframe`, `object`, `embed`, `form`, `input`, `button`, `textarea`, `select`, `meta`, and `link` elements.
- Remove all event handler attributes such as `onclick`.
- Remove dangerous URL protocols including `javascript:`, `vbscript:`, and non-image `data:` URLs.
- Add `rel="noopener noreferrer"` and `target="_blank"` to safe external anchors.
- Keep ordinary `http`, `https`, `mailto`, and inline `data:image/*` resources.

Sanitization can be implemented in the frontend with DOM APIs because the UI already runs in a browser environment and tests use jsdom. The backend remains responsible for extracting HTML and inline parts correctly.

### Detail Pane UX

Replace the field-grid feel with a compact message header:

- Subject remains prominent.
- Sender line shows display name and email separately when possible.
- `To` and `Cc` recipients show as chips or compact rows, with wrapping that does not overflow.
- Time, size, UID, and Message-ID move into a secondary metadata row.
- Body area renders HTML inside a constrained `.body-html-block` when available, otherwise renders plaintext in the existing monospaced style.

The visual style must remain consistent with AgentMail's dense industrial UI and existing dark/archive-beige themes.

## Data Flow

1. IMAP sync fetches `BODY.PEEK[]`.
2. `mail-protocol` parses subject, addresses, plaintext, HTML, inline CID resources, attachment metadata, and raw bytes into `MailMessage`.
3. `mail-store` upserts the message, FTS content, attachment metadata, and raw MIME source in one transaction.
4. `app-api` returns normal message list/detail payloads with `html_body` and inline resource data needed for display, but not raw MIME.
5. React detail pane builds a CID replacement map, sanitizes the HTML, and renders it. If no HTML body exists, it renders plaintext.
6. Permanent delete removes the message row and therefore the raw MIME row.

## Error Handling

- If raw MIME cannot be saved, the message upsert should fail rather than silently creating a partial local representation.
- If HTML parsing fails but plaintext exists, keep the message readable in plaintext.
- If sanitization removes all HTML content, fall back to plaintext.
- If a CID image is missing, leave the broken image hidden or replace it with a compact missing-inline indicator; do not fail the whole message render.
- If a permanent delete succeeds remotely but local raw cleanup fails, surface the local cleanup error because the user's requirement is no raw MIME residue after permanent delete.

## Testing

Backend tests:

- `mail-protocol` parses an HTML message and preserves both plaintext and HTML fields.
- `mail-protocol` extracts inline CID image parts with content ID, MIME type, and bytes.
- `mail-store` saves raw MIME on message upsert and can read back a presence/length indicator for tests.
- `mail-store` deletes raw MIME when a message row is permanently deleted.
- Existing sync reconciliation still excludes UID-less placeholders and does not treat raw MIME rows as independent messages.

Frontend tests:

- HTML body is preferred over plaintext when present.
- Remote image URLs survive sanitization.
- `cid:` URLs are replaced with inline image data URLs.
- Script tags, event attributes, forms, iframes, and dangerous link protocols are removed.
- Header display renders sender, recipients, cc, time, size, UID, and Message-ID without relying on the old metadata grid.

Verification commands:

```bash
pnpm test
pnpm build
pnpm rust:test
pnpm rust:check
cargo fmt --all --check
```

## Acceptance Criteria

- A synced remote message stores its raw MIME source locally.
- A Trash permanent delete or local permanent cleanup removes the message raw source.
- HTML messages display as HTML instead of flattened text.
- Remote images load by default in sanitized HTML.
- Inline CID images in HTML messages display.
- Plaintext-only messages still display correctly.
- The detail header presents mail identity fields in a polished mail-client layout.
- Ordinary attachments remain metadata-only and are not made downloadable in this phase.
