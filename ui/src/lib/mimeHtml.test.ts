import { describe, expect, it } from "vitest";
import type { InlineResource } from "../api";
import { buildRenderableHtml, buildRenderableMailHtml } from "./mimeHtml";

const logo: InlineResource = {
  id: "inline-1",
  message_id: "msg-1",
  content_id: "<logo@example.com>",
  filename: "logo.png",
  mime_type: "image/png",
  bytes: Array.from(new TextEncoder().encode("image-bytes"))
};

const textAttachment: InlineResource = {
  id: "inline-2",
  message_id: "msg-1",
  content_id: "<note@example.com>",
  filename: "note.txt",
  mime_type: "text/plain",
  bytes: Array.from(new TextEncoder().encode("plain-text"))
};

describe("buildRenderableHtml", () => {
  it("keeps remote images and replaces CID image sources with data URLs", () => {
    const rendered = buildRenderableHtml(
      '<p>Hello</p><img src="https://cdn.example.com/logo.png"><img src="cid:logo@example.com">',
      [logo]
    );

    const document = new DOMParser().parseFromString(rendered, "text/html");
    const images = Array.from(document.querySelectorAll("img"));

    expect(images[0].getAttribute("src")).toBe("https://cdn.example.com/logo.png");
    expect(images[1].getAttribute("src")).toBe("data:image/png;base64,aW1hZ2UtYnl0ZXM=");
  });

  it("preserves authored CSS including imports, font faces, media rules, and background URLs", () => {
    const rendered = buildRenderableHtml(
      `
        <html>
          <head>
            <style>
              @import url("https://cdn.example.com/base.css");
              @font-face { font-family: MailFont; src: url("https://cdn.example.com/font.woff2"); }
              .mail { width: 640px; background: #fff url("https://cdn.example.com/bg.png"); }
              @media screen and (max-width: 640px) { .mail { width: 100% !important; } }
            </style>
          </head>
          <body>
            <table class="mail" background="https://cdn.example.com/panel.png" style="background-image: url('https://cdn.example.com/card.png'); width: 640px;">
              <tr><td>body</td></tr>
            </table>
          </body>
        </html>
      `,
      []
    );
    const document = new DOMParser().parseFromString(rendered, "text/html");
    const styleText = document.querySelector("style")?.textContent ?? "";
    const table = document.querySelector("table");

    expect(styleText).toContain('@import url("https://cdn.example.com/base.css")');
    expect(styleText).toContain("@font-face");
    expect(styleText).toContain("font.woff2");
    expect(styleText).toContain('background: #fff url("https://cdn.example.com/bg.png")');
    expect(styleText).toContain("@media screen and (max-width: 640px)");
    expect(table?.getAttribute("background")).toBe("https://cdn.example.com/panel.png");
    expect(table?.getAttribute("style")).toContain("background-image: url('https://cdn.example.com/card.png')");
    expect(table?.getAttribute("style")).toContain("width: 640px");
  });

  it("keeps link stylesheet elements in head output", () => {
    const rendered = buildRenderableMailHtml(
      `
        <html>
          <head>
            <link rel="stylesheet" href="https://cdn.example.com/newsletter.css">
            <link rel="STYLESHEET" href="https://cdn.example.com/case.css">
            <link rel="preload stylesheet" href="https://cdn.example.com/preloaded.css" as="style">
            <link rel="preload" href="https://cdn.example.com/font.woff2" as="font">
          </head>
          <body><p>body</p></body>
        </html>
      `,
      []
    );

    expect(rendered.headStyles).toContain('<link rel="stylesheet" href="https://cdn.example.com/newsletter.css">');
    expect(rendered.headStyles).toContain('<link rel="STYLESHEET" href="https://cdn.example.com/case.css">');
    expect(rendered.headStyles).toContain(
      '<link rel="preload stylesheet" href="https://cdn.example.com/preloaded.css" as="style">'
    );
    expect(rendered.headStyles).not.toContain('rel="preload"');
    expect(rendered.bodyHtml).toContain("<p>body</p>");
  });

  it("removes scripts, event handler attributes, and javascript URLs", () => {
    const rendered = buildRenderableHtml(
      `
        <a id="safe" href="https://example.com/path">safe</a>
        <a id="mail" href="mailto:ops@example.com">mail</a>
        <a id="danger" href="javascript:alert(1)" onclick="alert(2)">danger</a>
        <img id="bad-img" src="javascript:alert(3)" onerror="alert(4)">
        <p onmouseover="alert(5)">body</p>
        <script>alert("owned")</script>
      `,
      []
    );

    const document = new DOMParser().parseFromString(rendered, "text/html");
    const safe = document.querySelector<HTMLAnchorElement>("#safe");
    const mail = document.querySelector<HTMLAnchorElement>("#mail");
    const danger = document.querySelector<HTMLAnchorElement>("#danger");
    const badImage = document.querySelector<HTMLImageElement>("#bad-img");

    expect(document.querySelector("script")).toBeNull();
    expect(document.querySelector("[onclick]")).toBeNull();
    expect(document.querySelector("[onerror]")).toBeNull();
    expect(document.querySelector("[onmouseover]")).toBeNull();
    expect(danger?.hasAttribute("href")).toBe(false);
    expect(badImage?.hasAttribute("src")).toBe(false);
    expect(safe?.getAttribute("href")).toBe("https://example.com/path");
    expect(safe?.getAttribute("target")).toBe("_blank");
    expect(safe?.getAttribute("rel")).toBe("noopener noreferrer");
    expect(mail?.getAttribute("href")).toBe("mailto:ops@example.com");
    expect(mail?.getAttribute("target")).toBe("_blank");
    expect(mail?.getAttribute("rel")).toBe("noopener noreferrer");
  });

  it("removes javascript URLs from namespaced href-like attributes", () => {
    const rendered = buildRenderableHtml(
      `
        <svg>
          <a id="bad" xlink:href="java&#115;cript:alert(1)">bad</a>
          <a id="safe" xlink:href="https://example.com/icon">safe</a>
        </svg>
      `,
      []
    );
    const document = new DOMParser().parseFromString(rendered, "text/html");

    expect(document.querySelector("#bad")?.hasAttribute("xlink:href")).toBe(false);
    expect(document.querySelector("#safe")?.getAttribute("xlink:href")).toBe("https://example.com/icon");
  });

  it("removes active embedded document and plugin elements while preserving form visuals", () => {
    const rendered = buildRenderableHtml(
      `
        <p>before</p>
        <iframe src="https://evil.example.com"></iframe>
        <object data="https://evil.example.com/plugin"></object>
        <embed src="https://evil.example.com/plugin">
        <form action="https://example.com/submit" method="post">
          <input name="email" value="me@example.com">
          <button formaction="https://example.com/button">Send</button>
          <textarea name="notes">hello</textarea>
          <select name="choice"><option selected>one</option></select>
        </form>
        <p>after</p>
      `,
      []
    );
    const document = new DOMParser().parseFromString(rendered, "text/html");
    const form = document.querySelector("form");

    expect(document.querySelector("iframe")).toBeNull();
    expect(document.querySelector("object")).toBeNull();
    expect(document.querySelector("embed")).toBeNull();
    expect(form).not.toBeNull();
    expect(form?.hasAttribute("action")).toBe(false);
    expect(form?.getAttribute("method")).toBe("post");
    expect(document.querySelector("input")?.getAttribute("value")).toBe("me@example.com");
    expect(document.querySelector("button")?.hasAttribute("formaction")).toBe(false);
    expect(document.querySelector("textarea")?.textContent).toBe("hello");
    expect(document.querySelector("select option")?.textContent).toBe("one");
    expect(document.body.textContent).toContain("before");
    expect(document.body.textContent).toContain("after");
  });

  it("matches CID resources case-insensitively with or without angle brackets", () => {
    const rendered = buildRenderableHtml('<img src="CID:<LOGO@EXAMPLE.COM>">', [logo]);
    const document = new DOMParser().parseFromString(rendered, "text/html");

    expect(document.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,aW1hZ2UtYnl0ZXM=");
  });

  it("does not turn non-image CID resources or normal attachments into injected content", () => {
    const rendered = buildRenderableHtml(
      '<p>body</p><img id="text-cid" src="cid:note@example.com"><a href="cid:note@example.com">attachment</a>',
      [textAttachment]
    );
    const document = new DOMParser().parseFromString(rendered, "text/html");

    expect(document.querySelector("#text-cid")?.hasAttribute("src")).toBe(false);
    expect(document.querySelector("a")?.getAttribute("href")).toBe("cid:note@example.com");
    expect(rendered).not.toContain("plain-text");
    expect(rendered).not.toContain("data:text/plain");
  });

  it("preserves safe body attributes in structured renderable output", () => {
    const rendered = buildRenderableMailHtml(
      `
        <html>
          <head><style>.body .card { width: 640px; }</style></head>
          <body class="body" style="background-color: #fff; color: #111;" onclick="alert(1)">
            <div class="card">body</div>
          </body>
        </html>
      `,
      []
    );
    const bodyAttributeMap = new Map(rendered.bodyAttributes.map((attribute) => [attribute.name, attribute.value]));

    expect(rendered.headStyles).toContain(".body .card");
    expect(bodyAttributeMap.get("class")).toBe("body");
    expect(bodyAttributeMap.get("style")).toContain("background-color: #fff; color: #111");
    expect(bodyAttributeMap.has("onclick")).toBe(false);
    const document = new DOMParser().parseFromString(rendered.bodyHtml, "text/html");
    expect(document.querySelector(".card")?.textContent?.trim()).toBe("body");
  });

  it("keeps compatible wrapper output for legacy callers", () => {
    const rendered = buildRenderableHtml('<body class="body"><div class="card">body</div></body>', []);
    const document = new DOMParser().parseFromString(rendered, "text/html");

    expect(document.querySelector("body > div.body .card")?.textContent).toBe("body");
  });
});
