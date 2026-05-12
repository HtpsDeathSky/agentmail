import type { InlineResource } from "../api";

const BLOCKED_ELEMENTS = new Set(["script", "iframe", "object", "embed"]);
const FORM_SUBMIT_ATTRIBUTES = new Set(["action", "formaction"]);
const CID_PREFIX = "cid:";

export interface RenderableHtmlAttribute {
  name: string;
  value: string;
}

export interface RenderableMailHtml {
  headStyles: string;
  bodyAttributes: RenderableHtmlAttribute[];
  bodyHtml: string;
}

export function buildRenderableMailHtml(html: string, inlineResources: InlineResource[]): RenderableMailHtml {
  const document = new DOMParser().parseFromString(html, "text/html");
  const cidResources = buildCidResourceMap(inlineResources);

  replaceCidSources(document, cidResources);
  prepareDocument(document);

  return {
    headStyles: serializeHeadStyles(document),
    bodyAttributes: serializeBodyAttributes(document),
    bodyHtml: document.body.innerHTML
  };
}

export function buildRenderableHtml(html: string, inlineResources: InlineResource[]): string {
  const renderable = buildRenderableMailHtml(html, inlineResources);
  return `${renderable.headStyles}${serializeBodyContent(renderable)}`;
}

export function serializeHtmlAttributes(attributes: RenderableHtmlAttribute[]) {
  const serialized = attributes
    .filter((attribute) => isSafeHtmlAttributeName(attribute.name))
    .map((attribute) => `${attribute.name}="${escapeHtmlAttribute(attribute.value)}"`)
    .join(" ");

  return serialized ? ` ${serialized}` : "";
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

function prepareDocument(document: Document) {
  for (const tagName of BLOCKED_ELEMENTS) {
    for (const element of Array.from(document.querySelectorAll(tagName))) {
      element.remove();
    }
  }

  for (const element of Array.from(document.querySelectorAll<HTMLElement>("*"))) {
    prepareAttributes(element);
  }
}

function prepareAttributes(element: HTMLElement) {
  for (const attribute of Array.from(element.attributes)) {
    const name = attribute.name.toLowerCase();

    if (name.startsWith("on")) {
      element.removeAttribute(attribute.name);
      continue;
    }

    if (FORM_SUBMIT_ATTRIBUTES.has(name)) {
      element.removeAttribute(attribute.name);
      continue;
    }

    if (isJavaScriptUrlAttribute(name) && isJavascriptUrl(attribute.value)) {
      element.removeAttribute(attribute.name);
    }
  }

  if (element instanceof HTMLAnchorElement && element.hasAttribute("href")) {
    element.setAttribute("target", "_blank");
    element.setAttribute("rel", "noopener noreferrer");
  }
}

function isJavaScriptUrlAttribute(name: string) {
  return name === "src" || name === "href" || name.endsWith(":href");
}

function isJavascriptUrl(value: string) {
  return value.trim().replace(/[\u0000-\u001f\u007f\s]+/g, "").toLowerCase().startsWith("javascript:");
}

function serializeHeadStyles(document: Document) {
  return Array.from(document.head.querySelectorAll<HTMLStyleElement | HTMLLinkElement>("style, link"))
    .filter((element) => element instanceof HTMLStyleElement || isStylesheetLink(element))
    .map((element) => element.outerHTML)
    .join("");
}

function isStylesheetLink(element: HTMLLinkElement) {
  return (element.getAttribute("rel") ?? "")
    .split(/\s+/)
    .some((token) => token.toLowerCase() === "stylesheet");
}

function serializeBodyAttributes(document: Document): RenderableHtmlAttribute[] {
  return Array.from(document.body.attributes).map((attribute) => ({
    name: attribute.name,
    value: attribute.value
  }));
}

function serializeBodyContent(renderable: RenderableMailHtml) {
  if (renderable.bodyAttributes.length === 0) return renderable.bodyHtml;
  return `<div${serializeHtmlAttributes(renderable.bodyAttributes)}>${renderable.bodyHtml}</div>`;
}

function isSafeHtmlAttributeName(name: string) {
  return /^[^\s"'>/=]+$/.test(name);
}

function escapeHtmlAttribute(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
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
