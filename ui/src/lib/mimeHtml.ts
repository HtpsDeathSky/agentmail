import type { InlineResource } from "../api";

const BLOCKED_ELEMENTS = new Set([
  "script",
  "iframe",
  "object",
  "embed",
  "form",
  "input",
  "button",
  "textarea",
  "select",
  "style",
  "meta",
  "link",
  "svg",
  "math"
]);

const CID_PREFIX = "cid:";
const BLOCKED_URL_ATTRIBUTES = new Set(["action", "background", "cite", "formaction", "poster"]);

export function buildRenderableHtml(html: string, inlineResources: InlineResource[]): string {
  const document = new DOMParser().parseFromString(html, "text/html");
  const cidResources = buildCidResourceMap(inlineResources);

  replaceCidSources(document, cidResources);
  sanitizeDocument(document);

  return document.body.innerHTML;
}

function buildCidResourceMap(inlineResources: InlineResource[]) {
  const resources = new Map<string, InlineResource>();

  for (const resource of inlineResources) {
    const normalized = normalizeContentId(resource.content_id);
    if (normalized) resources.set(normalized, resource);
  }

  return resources;
}

function replaceCidSources(document: Document, cidResources: Map<string, InlineResource>) {
  for (const element of Array.from(document.querySelectorAll<HTMLElement>("[src]"))) {
    const src = element.getAttribute("src")?.trim() ?? "";
    if (!src.toLowerCase().startsWith(CID_PREFIX)) continue;

    const cid = normalizeContentId(src.slice(CID_PREFIX.length));
    const resource = cidResources.get(cid);
    if (!resource || !isImageMimeType(resource.mime_type)) {
      element.removeAttribute("src");
      continue;
    }

    element.setAttribute("src", `data:${resource.mime_type};base64,${bytesToBase64(resource.bytes)}`);
  }
}

function sanitizeDocument(document: Document) {
  for (const tagName of BLOCKED_ELEMENTS) {
    for (const element of Array.from(document.querySelectorAll(tagName))) {
      element.remove();
    }
  }

  for (const element of Array.from(document.body.querySelectorAll<HTMLElement>("*"))) {
    sanitizeAttributes(element);
  }
}

function sanitizeAttributes(element: HTMLElement) {
  for (const attribute of Array.from(element.attributes)) {
    const name = attribute.name.toLowerCase();

    if (name.startsWith("on") || name === "style") {
      element.removeAttribute(attribute.name);
      continue;
    }

    if (name === "href" && !isSafeHref(attribute.value)) {
      element.removeAttribute(attribute.name);
      continue;
    }

    if (name === "src" && !isSafeSrc(element, attribute.value)) {
      element.removeAttribute(attribute.name);
      continue;
    }

    if (isBlockedUrlAttribute(name)) {
      element.removeAttribute(attribute.name);
    }
  }

  if (element instanceof HTMLAnchorElement && element.hasAttribute("href")) {
    element.setAttribute("target", "_blank");
    element.setAttribute("rel", "noopener noreferrer");
  }
}

function isBlockedUrlAttribute(name: string) {
  if (name === "href" || name === "src") return false;
  return name.includes("href") || name.includes("src") || BLOCKED_URL_ATTRIBUTES.has(name);
}

function isSafeHref(value: string) {
  const protocol = extractProtocol(value);
  return protocol === "http:" || protocol === "https:" || protocol === "mailto:";
}

function isSafeSrc(element: HTMLElement, value: string) {
  if (!(element instanceof HTMLImageElement)) return false;

  const protocol = extractProtocol(value);

  if (protocol === "http:" || protocol === "https:") return true;
  return protocol === "data:" && /^data:image\/[a-z0-9.+-]+;base64,/i.test(value.trim());
}

function extractProtocol(value: string) {
  const trimmed = value.trim();
  const colonIndex = trimmed.indexOf(":");
  if (colonIndex < 0) return "";
  return trimmed.slice(0, colonIndex + 1).toLowerCase();
}

function normalizeContentId(value: string) {
  const trimmed = value.trim();
  const withoutBrackets = trimmed.startsWith("<") && trimmed.endsWith(">") ? trimmed.slice(1, -1) : trimmed;
  return withoutBrackets.trim().toLowerCase();
}

function isImageMimeType(value: string) {
  return value.toLowerCase().startsWith("image/");
}

function bytesToBase64(bytes: number[]) {
  const chunkSize = 0x8000;
  let binary = "";

  for (let index = 0; index < bytes.length; index += chunkSize) {
    const chunk = bytes.slice(index, index + chunkSize);
    binary += String.fromCharCode(...chunk);
  }

  return btoa(binary);
}
