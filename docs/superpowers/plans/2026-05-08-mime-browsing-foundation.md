# MIME Browsing Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve raw MIME for synced messages and improve message browsing with sanitized HTML, remote images, inline CID images, and a better mail-style header.

**Architecture:** Extend the shared message model with raw MIME, HTML body, and inline CID resources. Store raw MIME in a dedicated SQLite table with cascade cleanup, keep ordinary attachment files out of scope, and render sanitized HTML in the existing React detail pane.

**Tech Stack:** Rust workspace (`mail-core`, `mail-protocol`, `mail-store`, `app-api`), SQLite via `rusqlite`, `mail_parser`, React/Vite/TypeScript, Vitest/jsdom, Tauri command bridge.

---

## File Structure

- Modify `crates/mail-core/src/lib.rs`: add `InlineResource` and new optional MIME browsing fields to `MailMessage`.
- Modify `crates/mail-protocol/src/lib.rs`: preserve raw MIME, parse HTML body, extract inline CID resources, and test parser behavior.
- Modify `crates/mail-store/src/lib.rs`: create `message_raw_sources`, persist raw MIME in upsert transactions, add test helpers, and verify cleanup on deletes.
- Modify `crates/app-api/src/lib.rs`: ensure API serialization and tests handle new fields without exposing raw MIME to normal frontend payloads if a view conversion is needed.
- Modify `ui/src/api.ts`: add `html_body` and `inline_resources` types to `MailMessage`.
- Create `ui/src/lib/mimeHtml.ts`: CID replacement and HTML sanitization helpers.
- Modify `ui/src/App.tsx`: replace the old body/header rendering with mail-style header rendering and HTML/plaintext body selection.
- Modify `ui/src/styles/app.css`: add styles for the improved message header and HTML body container.
- Modify `ui/src/App.test.ts` or create `ui/src/lib/mimeHtml.test.ts`: cover sanitizer, CID replacement, and rendering preference.

## Task 1: Shared MIME Model and Protocol Parsing

**Files:**
- Modify: `crates/mail-core/src/lib.rs`
- Modify: `crates/mail-protocol/src/lib.rs`

- [ ] **Step 1: Write failing parser tests**

Add tests in `crates/mail-protocol/src/lib.rs` under `mod tests`:

```rust
#[test]
fn parses_html_body_without_losing_plaintext() {
    let account = test_account();
    let folder = test_folder(&account);
    let raw = b"From: Sender <sender@example.com>\r\nTo: Ops <ops@example.com>\r\nSubject: HTML and text\r\nMessage-ID: <html-text@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=alt\r\n\r\n--alt\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlain fallback\r\n--alt\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><h1>HTML Body</h1><img src=\"https://example.com/pixel.png\"></body></html>\r\n--alt--\r\n";

    let parsed = parse_message(&account, &folder, 77, raw, Some(raw.len() as u32)).unwrap();

    assert_eq!(parsed.body.as_deref(), Some("Plain fallback"));
    assert!(parsed.html_body.as_deref().unwrap_or_default().contains("HTML Body"));
    assert_eq!(parsed.raw_mime.as_deref(), Some(raw.as_slice()));
}

#[test]
fn extracts_inline_cid_image_resource() {
    let account = test_account();
    let folder = test_folder(&account);
    let raw = b"From: Sender <sender@example.com>\r\nTo: Ops <ops@example.com>\r\nSubject: CID image\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=rel\r\n\r\n--rel\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><img src=\"cid:logo@example.com\"></body></html>\r\n--rel\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: base64\r\nContent-ID: <logo@example.com>\r\nContent-Disposition: inline; filename=logo.png\r\n\r\naW1hZ2UtYnl0ZXM=\r\n--rel--\r\n";

    let parsed = parse_message(&account, &folder, 78, raw, Some(raw.len() as u32)).unwrap();

    assert_eq!(parsed.inline_resources.len(), 1);
    assert_eq!(parsed.inline_resources[0].content_id, "logo@example.com");
    assert_eq!(parsed.inline_resources[0].mime_type, "image/png");
    assert_eq!(parsed.inline_resources[0].bytes, b"image-bytes");
}
```

If `test_account()` and `test_folder()` do not exist, extract them from the repeated account/folder setup already used by parser tests in the same module.

- [ ] **Step 2: Run parser tests to verify they fail**

Run:

```bash
cargo test -p mail-protocol parses_html_body_without_losing_plaintext extracts_inline_cid_image_resource
```

Expected: fail because `MailMessage.html_body`, `MailMessage.raw_mime`, `MailMessage.inline_resources`, and `InlineResource` do not exist.

- [ ] **Step 3: Add shared MIME fields**

In `crates/mail-core/src/lib.rs`, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InlineResource {
    pub id: String,
    pub message_id: String,
    pub content_id: String,
    pub filename: Option<String>,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}
```

Update `MailMessage`:

```rust
pub html_body: Option<String>,
pub raw_mime: Option<Vec<u8>>,
pub inline_resources: Vec<InlineResource>,
```

Place `html_body` near `body`, `raw_mime` near parsed content fields, and `inline_resources` near `attachments`.

- [ ] **Step 4: Update existing Rust constructors**

Search:

```bash
rg -n "MailMessage \\{|attachments: Vec::new\\(\\)|attachments: vec!\\[\\]" crates
```

For every `MailMessage` literal, add:

```rust
html_body: None,
raw_mime: None,
inline_resources: Vec::new(),
```

For live parsed messages, set real values in Step 5.

- [ ] **Step 5: Implement protocol parsing**

In `parse_message`, keep plaintext body as the AI/search body, capture HTML separately, preserve raw bytes, and extract inline resources:

```rust
let plain_body = parsed.body_text(0).map(|value| normalize_text(&value.into_owned()));
let html_body = parsed.body_html(0).map(|value| value.into_owned());
let body = plain_body
    .or_else(|| html_body.as_ref().map(|value| normalize_text(&html_to_text(value))))
    .unwrap_or_default();
let message_id = format!("{}:{}:{uid}", account.id, folder.id);
let inline_resources = parsed
    .parts
    .iter()
    .enumerate()
    .filter_map(|(index, part)| inline_resource_from_part(&message_id, index, part))
    .collect::<Vec<_>>();
```

Add helper functions matching the local `mail_parser` API. If direct `parsed.parts` access is unavailable, use the crate's exposed attachment/inline iterators and keep the tests as the contract. The helper must normalize `Content-ID` by trimming whitespace and surrounding `< >`.

In the returned `MailMessage`, set:

```rust
body: Some(body),
html_body,
raw_mime: Some(raw.to_vec()),
inline_resources,
```

- [ ] **Step 6: Run parser tests to verify they pass**

Run:

```bash
cargo test -p mail-protocol parses_html_body_without_losing_plaintext extracts_inline_cid_image_resource
```

Expected: pass.

- [ ] **Step 7: Run Rust check for model propagation**

Run:

```bash
pnpm rust:check
```

Expected: pass. If constructors are missing new fields, add the same default fields from Step 4.

## Task 2: Raw MIME Store Persistence and Cleanup

**Files:**
- Modify: `crates/mail-store/src/lib.rs`

- [ ] **Step 1: Write failing store tests**

Add tests in `crates/mail-store/src/lib.rs` under `mod tests`:

```rust
#[test]
fn upsert_message_persists_raw_mime_source() {
    let store = MailStore::memory().unwrap();
    let account = sample_account();
    let folder = sample_folder(&account, FolderRole::Inbox);
    store.save_account(&account).unwrap();
    store.save_folders(&[folder.clone()]).unwrap();
    let mut message = sample_message(&account, &folder, "42");
    message.raw_mime = Some(b"From: a@example.com\r\n\r\nbody".to_vec());

    store.save_messages(&[message.clone()]).unwrap();

    assert_eq!(
        store.raw_mime_len_for_test(&message.id).unwrap(),
        Some(b"From: a@example.com\r\n\r\nbody".len())
    );
}

#[test]
fn deleting_message_removes_raw_mime_source() {
    let store = MailStore::memory().unwrap();
    let account = sample_account();
    let folder = sample_folder(&account, FolderRole::Trash);
    store.save_account(&account).unwrap();
    store.save_folders(&[folder.clone()]).unwrap();
    let mut message = sample_message(&account, &folder, "99");
    message.raw_mime = Some(b"raw".to_vec());
    store.save_messages(&[message.clone()]).unwrap();

    store.delete_message_for_test(&message.id).unwrap();

    assert_eq!(store.raw_mime_len_for_test(&message.id).unwrap(), None);
}
```

If sample helpers have different names, use existing test helper patterns in this file.

- [ ] **Step 2: Run store tests to verify they fail**

Run:

```bash
cargo test -p mail-store upsert_message_persists_raw_mime_source deleting_message_removes_raw_mime_source
```

Expected: fail because the raw-source table and test helpers do not exist.

- [ ] **Step 3: Add raw-source migration**

In `migrate()`, after `messages` table creation, add:

```sql
CREATE TABLE IF NOT EXISTS message_raw_sources (
  message_id TEXT PRIMARY KEY,
  raw_mime BLOB NOT NULL,
  stored_at TEXT NOT NULL,
  FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE
);
```

- [ ] **Step 4: Persist raw MIME in `upsert_message_tx`**

After determining `stored_message_id`, write raw MIME in the same transaction:

```rust
if let Some(raw_mime) = message.raw_mime.as_deref() {
    tx.execute(
        r#"
        INSERT INTO message_raw_sources (message_id, raw_mime, stored_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(message_id) DO UPDATE SET
          raw_mime=excluded.raw_mime,
          stored_at=excluded.stored_at
        "#,
        params![stored_message_id, raw_mime, now_rfc3339()],
    )?;
}
```

Do not delete an existing raw source when a local placeholder with `raw_mime = None` updates unrelated fields.

- [ ] **Step 5: Add test-only helpers**

Inside `impl MailStore`, gated with `#[cfg(test)]`, add:

```rust
pub fn raw_mime_len_for_test(&self, message_id: &str) -> StoreResult<Option<usize>> {
    let conn = self.conn.lock();
    let len = conn
        .query_row(
            "SELECT length(raw_mime) FROM message_raw_sources WHERE message_id = ?1",
            params![message_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    Ok(len.map(|value| value as usize))
}

pub fn delete_message_for_test(&self, message_id: &str) -> StoreResult<()> {
    let conn = self.conn.lock();
    conn.execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
    Ok(())
}
```

- [ ] **Step 6: Ensure row conversion handles new fields**

In `message_from_row`, set fields that are not selected from the normal message table:

```rust
html_body,
raw_mime: None,
inline_resources,
```

If `html_body` and `inline_resources` are stored in `messages`, select and deserialize them. If they are not stored in this task, add storage in Task 3 before frontend work.

- [ ] **Step 7: Run store tests to verify they pass**

Run:

```bash
cargo test -p mail-store upsert_message_persists_raw_mime_source deleting_message_removes_raw_mime_source
```

Expected: pass.

## Task 3: Persist HTML Body and Inline Resources Through Store/API

**Files:**
- Modify: `crates/mail-store/src/lib.rs`
- Modify: `crates/app-api/src/lib.rs`
- Modify: `ui/src/api.ts`

- [ ] **Step 1: Write failing store round-trip test**

Add a `mail-store` test:

```rust
#[test]
fn message_round_trip_preserves_html_body_and_inline_resources() {
    let store = MailStore::memory().unwrap();
    let account = sample_account();
    let folder = sample_folder(&account, FolderRole::Inbox);
    store.save_account(&account).unwrap();
    store.save_folders(&[folder.clone()]).unwrap();
    let mut message = sample_message(&account, &folder, "123");
    message.html_body = Some("<p>Hello <img src=\"cid:logo@example.com\"></p>".to_string());
    message.inline_resources = vec![InlineResource {
        id: "inline-1".to_string(),
        message_id: message.id.clone(),
        content_id: "logo@example.com".to_string(),
        filename: Some("logo.png".to_string()),
        mime_type: "image/png".to_string(),
        bytes: b"image".to_vec(),
    }];

    store.save_messages(&[message.clone()]).unwrap();
    let loaded = store.get_message(&message.id).unwrap();

    assert_eq!(loaded.html_body, message.html_body);
    assert_eq!(loaded.inline_resources, message.inline_resources);
    assert_eq!(loaded.raw_mime, None);
}
```

Ensure `InlineResource` is imported from `mail_core`.

- [ ] **Step 2: Run round-trip test to verify it fails**

Run:

```bash
cargo test -p mail-store message_round_trip_preserves_html_body_and_inline_resources
```

Expected: fail because HTML and inline resources are not persisted or selected.

- [ ] **Step 3: Add message columns**

In migration, add idempotent schema migration:

```rust
add_column_if_missing(&conn, "messages", "html_body", "TEXT")?;
add_column_if_missing(&conn, "messages", "inline_resources_json", "TEXT NOT NULL DEFAULT '[]'")?;
```

If no helper exists, add a small private `add_column_if_missing` using `PRAGMA table_info(messages)` and `ALTER TABLE`.

- [ ] **Step 4: Update message insert/select/upsert**

In all `SELECT` statements returning messages, include:

```sql
m.html_body, m.inline_resources_json
```

or for unaliased selects:

```sql
html_body, inline_resources_json
```

In `upsert_message_tx`, serialize:

```rust
let inline_resources_json = serde_json::to_string(&message.inline_resources)?;
```

Add `html_body` and `inline_resources_json` to the insert, update, and conflict-update column lists.

- [ ] **Step 5: Update `message_from_row`**

Read the new columns after the existing fields:

```rust
let html_body: Option<String> = row.get(19)?;
let inline_resources_json: String = row.get(20)?;
let inline_resources =
    serde_json::from_str::<Vec<InlineResource>>(&inline_resources_json).unwrap_or_default();
```

Set `raw_mime: None` on loaded normal messages so raw bytes do not travel through list/detail APIs.

- [ ] **Step 6: Update app-api test constructors**

In `crates/app-api/src/lib.rs`, update every `MailMessage` literal with:

```rust
html_body: None,
raw_mime: None,
inline_resources: Vec::new(),
```

If app-api has frontend view structs, add:

```rust
pub html_body: Option<String>,
pub inline_resources: Vec<InlineResource>,
```

Do not include `raw_mime` in frontend-facing view structs.

- [ ] **Step 7: Update TypeScript API types**

In `ui/src/api.ts`, add:

```ts
export interface InlineResource {
  id: string;
  message_id: string;
  content_id: string;
  filename?: string | null;
  mime_type: string;
  bytes: number[];
}
```

Update `MailMessage`:

```ts
html_body?: string | null;
inline_resources: InlineResource[];
```

- [ ] **Step 8: Run round-trip and app checks**

Run:

```bash
cargo test -p mail-store message_round_trip_preserves_html_body_and_inline_resources
pnpm rust:check
pnpm build
```

Expected: all pass.

## Task 4: Frontend MIME HTML Sanitization and CID Replacement

**Files:**
- Create: `ui/src/lib/mimeHtml.ts`
- Create: `ui/src/lib/mimeHtml.test.ts`

- [ ] **Step 1: Write failing frontend helper tests**

Create `ui/src/lib/mimeHtml.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { buildRenderableHtml } from "./mimeHtml";
import type { InlineResource } from "../api";

const logo: InlineResource = {
  id: "inline-1",
  message_id: "msg-1",
  content_id: "logo@example.com",
  filename: "logo.png",
  mime_type: "image/png",
  bytes: Array.from(new TextEncoder().encode("image-bytes")),
};

describe("buildRenderableHtml", () => {
  it("keeps remote images and replaces cid images", () => {
    const html = buildRenderableHtml(
      `<p>Hello</p><img src="https://example.com/pixel.png"><img src="cid:logo@example.com">`,
      [logo],
    );

    expect(html).toContain(`src="https://example.com/pixel.png"`);
    expect(html).toContain(`src="data:image/png;base64,`);
    expect(html).not.toContain("cid:logo@example.com");
  });

  it("removes scripts event handlers forms iframes and dangerous links", () => {
    const html = buildRenderableHtml(
      `<script>alert(1)</script><p onclick="alert(2)">Hi</p><form><input></form><iframe src="https://evil.example"></iframe><a href="javascript:alert(3)">bad</a><a href="https://safe.example">safe</a>`,
      [],
    );

    expect(html).not.toContain("<script");
    expect(html).not.toContain("onclick");
    expect(html).not.toContain("<form");
    expect(html).not.toContain("<iframe");
    expect(html).not.toContain("javascript:");
    expect(html).toContain(`href="https://safe.example"`);
    expect(html).toContain(`target="_blank"`);
    expect(html).toContain(`rel="noopener noreferrer"`);
  });
});
```

- [ ] **Step 2: Run helper tests to verify they fail**

Run:

```bash
pnpm test ui/src/lib/mimeHtml.test.ts
```

Expected: fail because `mimeHtml.ts` does not exist.

- [ ] **Step 3: Implement MIME HTML helper**

Create `ui/src/lib/mimeHtml.ts`:

```ts
import type { InlineResource } from "../api";

const BLOCKED_TAGS = new Set(["SCRIPT", "IFRAME", "OBJECT", "EMBED", "FORM", "INPUT", "BUTTON", "TEXTAREA", "SELECT", "META", "LINK"]);
const SAFE_URL_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);

export function buildRenderableHtml(html: string, inlineResources: InlineResource[]): string {
  const document = new DOMParser().parseFromString(html, "text/html");
  replaceCidSources(document, inlineResources);
  sanitizeDocument(document);
  return document.body.innerHTML;
}

function replaceCidSources(document: Document, inlineResources: InlineResource[]) {
  const cidMap = new Map<string, string>();
  for (const resource of inlineResources) {
    const normalized = normalizeCid(resource.content_id);
    if (!normalized || !resource.mime_type.startsWith("image/")) continue;
    cidMap.set(normalized, `data:${resource.mime_type};base64,${bytesToBase64(resource.bytes)}`);
  }

  document.querySelectorAll<HTMLElement>("[src]").forEach((element) => {
    const src = element.getAttribute("src") ?? "";
    if (!src.toLowerCase().startsWith("cid:")) return;
    const replacement = cidMap.get(normalizeCid(src.slice(4)));
    if (replacement) element.setAttribute("src", replacement);
    else element.removeAttribute("src");
  });
}

function sanitizeDocument(document: Document) {
  document.querySelectorAll("*").forEach((element) => {
    if (BLOCKED_TAGS.has(element.tagName)) {
      element.remove();
      return;
    }

    for (const attribute of Array.from(element.attributes)) {
      const name = attribute.name.toLowerCase();
      const value = attribute.value.trim();
      if (name.startsWith("on")) {
        element.removeAttribute(attribute.name);
        continue;
      }
      if ((name === "href" || name === "src") && !isSafeUrl(value, name)) {
        element.removeAttribute(attribute.name);
      }
    }

    if (element.tagName === "A") {
      const href = element.getAttribute("href");
      if (href) {
        element.setAttribute("target", "_blank");
        element.setAttribute("rel", "noopener noreferrer");
      }
    }
  });
}

function isSafeUrl(value: string, attributeName: string): boolean {
  if (value.startsWith("#")) return true;
  try {
    const parsed = new URL(value, "https://agentmail.local");
    if (SAFE_URL_PROTOCOLS.has(parsed.protocol)) return true;
    return attributeName === "src" && parsed.protocol === "data:" && value.toLowerCase().startsWith("data:image/");
  } catch {
    return false;
  }
}

function normalizeCid(value: string): string {
  return value.trim().replace(/^<|>$/g, "").toLowerCase();
}

function bytesToBase64(bytes: number[]): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}
```

- [ ] **Step 4: Run helper tests to verify they pass**

Run:

```bash
pnpm test ui/src/lib/mimeHtml.test.ts
```

Expected: pass.

## Task 5: Message Detail UI Browsing Improvements

**Files:**
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/styles/app.css`
- Modify: `ui/src/App.test.ts`
- Modify: `ui/src/data/demoBackend.ts`

- [ ] **Step 1: Write failing App rendering test**

In `ui/src/App.test.ts`, add or update a test that selects a demo message with `html_body`:

```ts
it("renders html message body and mail-style header fields", async () => {
  render(<App />);

  const htmlMessage = await screen.findByText("HTML newsletter preview");
  await userEvent.click(htmlMessage);

  expect(await screen.findByText("Sender")).toBeInTheDocument();
  expect(screen.getByText("sender@example.com")).toBeInTheDocument();
  expect(screen.getByText("ops@example.com")).toBeInTheDocument();
  expect(screen.getByText("HTML Body")).toBeInTheDocument();
  expect(document.querySelector(".body-html-block")).not.toBeNull();
  expect(document.querySelector(".metadata-grid")).toBeNull();
});
```

Adjust selectors to match existing test setup. The test must verify the old `.metadata-grid` is no longer used in the selected detail view.

- [ ] **Step 2: Run App test to verify it fails**

Run:

```bash
pnpm test ui/src/App.test.ts
```

Expected: fail because demo data and UI do not yet render HTML body/header layout.

- [ ] **Step 3: Add demo HTML message**

In `ui/src/data/demoBackend.ts`, add one inbox message with:

```ts
subject: "HTML newsletter preview",
sender: "Sender <sender@example.com>",
recipients: ["ops@example.com"],
cc: ["audit@example.com"],
body_preview: "HTML Body",
body: "Plain fallback",
html_body: `<section><h2>HTML Body</h2><p>Remote image follows.</p><img src="https://example.com/pixel.png" alt="pixel"></section>`,
inline_resources: [],
```

Update all existing demo messages with `html_body: null` and `inline_resources: []`.

- [ ] **Step 4: Implement header/body rendering**

In `ui/src/App.tsx`, import:

```ts
import { buildRenderableHtml } from "./lib/mimeHtml";
```

Add helpers near existing formatting helpers:

```ts
function splitMailbox(value: string) {
  const match = value.match(/^(.*?)\s*<([^>]+)>$/);
  if (!match) return { name: value, email: value.includes("@") ? value : "" };
  return { name: match[1].trim() || match[2], email: match[2].trim() };
}
```

Replace the old `.metadata-grid` and `<pre className="body-block">` block with:

```tsx
const sender = splitMailbox(selectedMessage.sender);
const renderableHtml = selectedMessage.html_body
  ? buildRenderableHtml(selectedMessage.html_body, selectedMessage.inline_resources)
  : null;
```

Render a `.message-envelope` containing sender identity, recipient rows, and secondary metadata. Render:

```tsx
{renderableHtml ? (
  <div className="body-html-block" dangerouslySetInnerHTML={{ __html: renderableHtml }} />
) : (
  <pre className="body-block">{selectedMessage.body ?? selectedMessage.body_preview}</pre>
)}
```

- [ ] **Step 5: Add CSS**

In `ui/src/styles/app.css`, add styles for:

```css
.message-envelope { ... }
.sender-card { ... }
.sender-avatar { ... }
.sender-identity { ... }
.recipient-rows { ... }
.recipient-row { ... }
.recipient-chip { ... }
.message-secondary-meta { ... }
.body-html-block { ... }
.body-html-block img { max-width: 100%; height: auto; }
.body-html-block a { color: var(--color-warning-bright); }
```

Use existing color variables and 3-4px radii. Keep text wrapping with `overflow-wrap: anywhere`.

- [ ] **Step 6: Run App test to verify it passes**

Run:

```bash
pnpm test ui/src/App.test.ts
```

Expected: pass.

- [ ] **Step 7: Run frontend build**

Run:

```bash
pnpm build
```

Expected: pass.

## Task 6: Integration Verification and Cleanup

**Files:**
- Modify only files needed to fix integration failures.

- [ ] **Step 1: Run full verification**

Run:

```bash
pnpm test
pnpm build
pnpm rust:test
pnpm rust:check
cargo fmt --all --check
```

Expected: all pass.

- [ ] **Step 2: Inspect diff for scope**

Run:

```bash
git diff --stat
git diff -- docs/superpowers/specs/2026-05-08-mime-browsing-foundation-design.md docs/superpowers/plans/2026-05-08-mime-browsing-foundation.md
```

Expected: changes are limited to MIME browsing foundation, docs, tests, and required model/store/UI integration.

- [ ] **Step 3: Update handoff docs if behavior changed**

If the implementation completes, update `docs/PROJECT_STATUS.md` and `docs/NEXT_STEPS.md` to say raw MIME is now stored and first-phase HTML/CID browsing is implemented, while ordinary attachment download remains later scope.

- [ ] **Step 4: Final branch commit**

Stage only intended files:

```bash
git add crates/mail-core/src/lib.rs crates/mail-protocol/src/lib.rs crates/mail-store/src/lib.rs crates/app-api/src/lib.rs ui/src/api.ts ui/src/lib/mimeHtml.ts ui/src/lib/mimeHtml.test.ts ui/src/App.tsx ui/src/App.test.ts ui/src/data/demoBackend.ts ui/src/styles/app.css docs/superpowers/specs/2026-05-08-mime-browsing-foundation-design.md docs/superpowers/plans/2026-05-08-mime-browsing-foundation.md docs/PROJECT_STATUS.md docs/NEXT_STEPS.md
git commit -m "feat: add mime browsing foundation"
```

Expected: local commit only. Do not push unless explicitly requested.
