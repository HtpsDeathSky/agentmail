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
  "meta",
  "link",
  "svg",
  "math"
]);

const CID_PREFIX = "cid:";
const BLOCKED_URL_ATTRIBUTES = new Set(["action", "background", "cite", "formaction", "poster"]);
const BLOCKED_CSS_PROPERTIES = new Set(["behavior", "-moz-binding"]);

export function buildRenderableHtml(html: string, inlineResources: InlineResource[]): string {
  const document = new DOMParser().parseFromString(html, "text/html");
  const cidResources = buildCidResourceMap(inlineResources);

  replaceCidSources(document, cidResources);
  sanitizeDocument(document);

  return `${serializeHeadStyles(document)}${serializeBodyContent(document)}`;
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

  for (const element of Array.from(document.querySelectorAll<HTMLElement>("style"))) {
    sanitizeStyleElement(element);
  }

  for (const element of Array.from(document.querySelectorAll<HTMLElement>("*"))) {
    sanitizeAttributes(element);
  }
}

function sanitizeAttributes(element: HTMLElement) {
  for (const attribute of Array.from(element.attributes)) {
    const name = attribute.name.toLowerCase();

    if (name.startsWith("on")) {
      element.removeAttribute(attribute.name);
      continue;
    }

    if (name === "style") {
      const sanitizedStyle = sanitizeStyleDeclarations(attribute.value);
      if (sanitizedStyle) {
        element.setAttribute(attribute.name, sanitizedStyle);
      } else {
        element.removeAttribute(attribute.name);
      }
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

    if (name === "background") {
      if (!isSafeCssUrl(attribute.value)) element.removeAttribute(attribute.name);
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

function serializeHeadStyles(document: Document) {
  return Array.from(document.head.querySelectorAll<HTMLStyleElement>("style"))
    .map((element) => element.outerHTML)
    .join("");
}

function serializeBodyContent(document: Document) {
  const bodyAttributes = Array.from(document.body.attributes);
  if (bodyAttributes.length === 0) return document.body.innerHTML;

  const wrapper = document.createElement("div");
  for (const attribute of bodyAttributes) {
    wrapper.setAttribute(attribute.name, attribute.value);
  }
  wrapper.innerHTML = document.body.innerHTML;

  return wrapper.outerHTML;
}

function sanitizeStyleElement(element: HTMLElement) {
  const sanitizedCss = sanitizeStyleSheet(element.textContent ?? "");
  if (!sanitizedCss) {
    element.remove();
    return;
  }

  element.textContent = sanitizedCss;
}

function sanitizeStyleSheet(css: string) {
  const withoutComments = stripCssComments(css);
  const withoutImports = removeCssImportRules(withoutComments);
  const sanitized = sanitizeCssRules(withoutImports).trim();

  if (normalizeCssForSafety(sanitized).includes("@import")) return "";
  return sanitized;
}

function sanitizeCssRules(css: string): string {
  let sanitized = "";
  let index = 0;

  while (index < css.length) {
    const openBraceIndex = css.indexOf("{", index);
    if (openBraceIndex < 0) break;

    const closeBraceIndex = findMatchingBrace(css, openBraceIndex);
    if (closeBraceIndex < 0) break;

    const prelude = css.slice(index, openBraceIndex).trim();
    const block = css.slice(openBraceIndex + 1, closeBraceIndex);
    const sanitizedBlock = hasTopLevelBrace(block) ? sanitizeCssRules(block) : sanitizeStyleDeclarations(block);

    if (prelude && sanitizedBlock && isSafeCssPrelude(prelude)) {
      sanitized += `${prelude} { ${sanitizedBlock} }\n`;
    }

    index = closeBraceIndex + 1;
  }

  return sanitized;
}

function sanitizeStyleDeclarations(css: string) {
  const declarations: string[] = [];

  for (const declaration of splitCssDeclarations(css)) {
    const colonIndex = findCssDeclarationColon(declaration);
    if (colonIndex < 0) continue;

    const property = declaration.slice(0, colonIndex).trim();
    const value = declaration.slice(colonIndex + 1).trim();
    if (!property || !value || !isSafeCssProperty(property) || !isSafeCssValue(value)) continue;

    declarations.push(`${property}: ${normalizeCssDeclarationValue(value)};`);
  }

  return declarations.join(" ");
}

function splitCssDeclarations(css: string) {
  const declarations: string[] = [];
  let start = 0;
  let depth = 0;
  let quote = "";
  let escaped = false;

  for (let index = 0; index < css.length; index += 1) {
    const char = css[index];

    if (escaped) {
      escaped = false;
      continue;
    }

    if (char === "\\") {
      escaped = true;
      continue;
    }

    if (quote) {
      if (char === quote) quote = "";
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }

    if (char === "(") {
      depth += 1;
      continue;
    }

    if (char === ")") {
      depth = Math.max(0, depth - 1);
      continue;
    }

    if (char === ";" && depth === 0) {
      declarations.push(css.slice(start, index));
      start = index + 1;
    }
  }

  declarations.push(css.slice(start));
  return declarations;
}

function findCssDeclarationColon(css: string) {
  let depth = 0;
  let quote = "";
  let escaped = false;

  for (let index = 0; index < css.length; index += 1) {
    const char = css[index];

    if (escaped) {
      escaped = false;
      continue;
    }

    if (char === "\\") {
      escaped = true;
      continue;
    }

    if (quote) {
      if (char === quote) quote = "";
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }

    if (char === "(") {
      depth += 1;
      continue;
    }

    if (char === ")") {
      depth = Math.max(0, depth - 1);
      continue;
    }

    if (char === ":" && depth === 0) return index;
  }

  return -1;
}

function isSafeCssProperty(property: string) {
  const normalized = decodeCssEscapes(property).trim().toLowerCase();
  return normalized.length > 0 && !BLOCKED_CSS_PROPERTIES.has(normalized);
}

function isSafeCssPrelude(prelude: string) {
  const normalized = normalizeCssForSafety(prelude);
  if (normalized.includes("@import")) return false;
  return isSafeCssValue(prelude);
}

function isSafeCssValue(value: string) {
  const normalized = normalizeCssForSafety(value);

  if (
    normalized.includes("javascript:") ||
    normalized.includes("vbscript:") ||
    normalized.includes("expression(") ||
    normalized.includes("behavior:") ||
    normalized.includes("-moz-binding")
  ) {
    return false;
  }

  const urls = extractCssUrls(value);
  return urls !== null && urls.every(isSafeCssUrl);
}

function normalizeCssDeclarationValue(value: string) {
  return stripCssComments(value).trim();
}

function stripCssComments(value: string) {
  return value.replace(/\/\*[\s\S]*?\*\//g, "");
}

function normalizeCssForSafety(value: string) {
  return decodeCssEscapes(stripCssComments(value))
    .replace(/[\u0000-\u001f\u007f\s]+/g, "")
    .toLowerCase();
}

function decodeCssEscapes(value: string) {
  return value.replace(/\\([0-9a-fA-F]{1,6}\s?|.)/g, (_match, escape: string) => {
    const hexMatch = escape.match(/^([0-9a-fA-F]{1,6})\s?$/);
    if (!hexMatch) return escape;

    const codePoint = Number.parseInt(hexMatch[1], 16);
    if (!Number.isFinite(codePoint) || codePoint <= 0) return "";

    try {
      return String.fromCodePoint(codePoint);
    } catch {
      return "";
    }
  });
}

function removeCssImportRules(css: string) {
  let sanitized = "";
  let index = 0;

  while (index < css.length) {
    if (isCssImportAt(css, index)) {
      index = findCssRuleEnd(css, index);
      continue;
    }

    sanitized += css[index];
    index += 1;
  }

  return sanitized;
}

function isCssImportAt(css: string, index: number) {
  if (css[index] !== "@") return false;
  return css.slice(index, index + 7).toLowerCase() === "@import";
}

function findCssRuleEnd(css: string, startIndex: number) {
  let quote = "";
  let escaped = false;

  for (let index = startIndex; index < css.length; index += 1) {
    const char = css[index];

    if (escaped) {
      escaped = false;
      continue;
    }

    if (char === "\\") {
      escaped = true;
      continue;
    }

    if (quote) {
      if (char === quote) quote = "";
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }

    if (char === ";") return index + 1;
    if (char === "{") {
      const closeBraceIndex = findMatchingBrace(css, index);
      return closeBraceIndex < 0 ? css.length : closeBraceIndex + 1;
    }
  }

  return css.length;
}

function findMatchingBrace(css: string, openBraceIndex: number) {
  let depth = 0;
  let quote = "";
  let escaped = false;

  for (let index = openBraceIndex; index < css.length; index += 1) {
    const char = css[index];

    if (escaped) {
      escaped = false;
      continue;
    }

    if (char === "\\") {
      escaped = true;
      continue;
    }

    if (quote) {
      if (char === quote) quote = "";
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }

    if (char === "{") {
      depth += 1;
      continue;
    }

    if (char === "}") {
      depth -= 1;
      if (depth === 0) return index;
    }
  }

  return -1;
}

function hasTopLevelBrace(css: string) {
  let quote = "";
  let escaped = false;

  for (const char of css) {
    if (escaped) {
      escaped = false;
      continue;
    }

    if (char === "\\") {
      escaped = true;
      continue;
    }

    if (quote) {
      if (char === quote) quote = "";
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }

    if (char === "{") return true;
  }

  return false;
}

function extractCssUrls(value: string) {
  const urls: string[] = [];
  const urlPattern = /url\s*\(/gi;
  let match: RegExpExecArray | null;

  while ((match = urlPattern.exec(value)) !== null) {
    let index = match.index + match[0].length;
    while (index < value.length && /\s/.test(value[index])) index += 1;

    const quote = value[index] === '"' || value[index] === "'" ? value[index] : "";
    if (quote) index += 1;

    const startIndex = index;
    let escaped = false;
    let url = "";

    for (; index < value.length; index += 1) {
      const char = value[index];

      if (escaped) {
        url += char;
        escaped = false;
        continue;
      }

      if (char === "\\") {
        url += char;
        escaped = true;
        continue;
      }

      if (quote) {
        if (char === quote) break;
        url += char;
        continue;
      }

      if (char === ")") break;
      url += char;
    }

    if (index >= value.length) return null;

    if (quote) {
      index += 1;
      while (index < value.length && /\s/.test(value[index])) index += 1;
      if (value[index] !== ")") return null;
    } else if (value[index] !== ")") {
      return null;
    }

    urls.push(value.slice(startIndex, startIndex + url.length));
    urlPattern.lastIndex = index + 1;
  }

  return urls;
}

function isSafeCssUrl(value: string) {
  const trimmed = decodeCssEscapes(stripCssComments(value)).trim().replace(/[\u0000-\u001f\u007f]/g, "");
  const compact = trimmed.replace(/\s+/g, "").toLowerCase();

  if (!trimmed || compact.startsWith("//")) return false;
  if (compact.startsWith("javascript:") || compact.startsWith("vbscript:")) return false;

  const protocol = extractProtocol(trimmed);

  if (protocol === "http:" || protocol === "https:") return true;
  if (protocol === "data:") return /^data:image\/[a-z0-9.+-]+;base64,/i.test(trimmed);
  return protocol === "";
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
