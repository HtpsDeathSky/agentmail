import { describe, expect, it } from "vitest";
import type { InlineResource } from "../api";
import { buildRenderableHtml } from "./mimeHtml";

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

  it("removes unsafe elements, event handlers, forms, iframes, and dangerous links", () => {
    const rendered = buildRenderableHtml(
      `
        <a id="safe" href="https://example.com/path">safe</a>
        <a id="mail" href="mailto:ops@example.com">mail</a>
        <a id="danger" href="javascript:alert(1)" onclick="alert(2)">danger</a>
        <form><input name="token"><button>send</button></form>
        <iframe src="https://evil.example.com"></iframe>
        <script>alert("owned")</script>
        <p onmouseover="alert(3)">body</p>
      `,
      []
    );

    const document = new DOMParser().parseFromString(rendered, "text/html");
    const safe = document.querySelector<HTMLAnchorElement>("#safe");
    const mail = document.querySelector<HTMLAnchorElement>("#mail");
    const danger = document.querySelector<HTMLAnchorElement>("#danger");

    expect(document.querySelector("script")).toBeNull();
    expect(document.querySelector("form")).toBeNull();
    expect(document.querySelector("input")).toBeNull();
    expect(document.querySelector("button")).toBeNull();
    expect(document.querySelector("iframe")).toBeNull();
    expect(document.querySelector("[onclick]")).toBeNull();
    expect(document.querySelector("[onmouseover]")).toBeNull();
    expect(danger?.hasAttribute("href")).toBe(false);
    expect(safe?.getAttribute("href")).toBe("https://example.com/path");
    expect(safe?.getAttribute("target")).toBe("_blank");
    expect(safe?.getAttribute("rel")).toBe("noopener noreferrer");
    expect(mail?.getAttribute("href")).toBe("mailto:ops@example.com");
  });

  it("matches CID resources case-insensitively with or without angle brackets", () => {
    const rendered = buildRenderableHtml('<img src="CID:<LOGO@EXAMPLE.COM>">', [logo]);
    const document = new DOMParser().parseFromString(rendered, "text/html");

    expect(document.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,aW1hZ2UtYnl0ZXM=");
  });

  it("keeps safe CSS while removing dangerous style content", () => {
    const rendered = buildRenderableHtml(
      `
        <html>
          <head>
            <style>
              @import url("https://tracker.example.com/mail.css");
              .mail { color: #111; padding: 12px; background-image: url("https://cdn.example.com/bg.png"); }
              .bad-js { background-image: url("javascript:alert(1)"); }
              .bad-vbs { background-image: url(vbscript:msgbox(1)); }
              .bad-expression { width: expression(alert(1)); }
              .bad-binding { -moz-binding: url("https://evil.example.com/xss.xml#xss"); }
              .bad-behavior { behavior: url("https://evil.example.com/ie.htc"); }
              @media screen and (max-width: 640px) { .mail { width: 100%; } }
            </style>
          </head>
          <body>
            <p class="mail" style="background: #fff url(https://cdn.example.com/card.png); color: #222; padding: 16px; width: 600px; background-image: url(javascript:alert(2)); behavior: url(https://evil.example.com/ie.htc);">body</p>
          </body>
        </html>
      `,
      []
    );
    const document = new DOMParser().parseFromString(rendered, "text/html");
    const styleText = document.querySelector("style")?.textContent ?? "";
    const inlineStyle = document.querySelector("p")?.getAttribute("style") ?? "";

    expect(document.querySelector("style")).not.toBeNull();
    expect(styleText).toContain(".mail");
    expect(styleText).toContain("color: #111;");
    expect(styleText).toContain("padding: 12px;");
    expect(styleText).toContain('background-image: url("https://cdn.example.com/bg.png");');
    expect(styleText).toContain("@media screen and (max-width: 640px)");
    expect(styleText).toContain("width: 100%;");
    expect(styleText).not.toContain("@import");
    expect(styleText).not.toContain("javascript:");
    expect(styleText).not.toContain("vbscript:");
    expect(styleText).not.toContain("expression");
    expect(styleText).not.toContain("-moz-binding");
    expect(styleText).not.toContain("behavior");
    expect(inlineStyle).toContain("background: #fff url(https://cdn.example.com/card.png);");
    expect(inlineStyle).toContain("color: #222;");
    expect(inlineStyle).toContain("padding: 16px;");
    expect(inlineStyle).toContain("width: 600px;");
    expect(inlineStyle).not.toContain("javascript:");
    expect(inlineStyle).not.toContain("behavior");
    expect(document.body.textContent).toContain("body");
  });

  it("includes sanitized head style rules in the renderable HTML", () => {
    const rendered = buildRenderableHtml(
      `
        <html>
          <head>
            <style>.mail-shell { max-width: 720px; margin: 0 auto; }</style>
          </head>
          <body><div class="mail-shell">body</div></body>
        </html>
      `,
      []
    );

    expect(rendered.trim().startsWith("<style>")).toBe(true);

    const document = new DOMParser().parseFromString(rendered, "text/html");
    expect(document.querySelector("style")?.textContent).toContain(".mail-shell");
    expect(document.querySelector(".mail-shell")?.textContent).toBe("body");
  });

  it("preserves safe body attributes on the renderable wrapper", () => {
    const rendered = buildRenderableHtml(
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
    const document = new DOMParser().parseFromString(rendered, "text/html");
    const wrapper = document.querySelector("body > div.body");

    expect(wrapper).not.toBeNull();
    expect(wrapper?.getAttribute("style")).toContain("background-color: #fff;");
    expect(wrapper?.getAttribute("style")).toContain("color: #111;");
    expect(wrapper?.hasAttribute("onclick")).toBe(false);
    expect(document.querySelector(".card")?.textContent?.trim()).toBe("body");
  });

  it("keeps safe inline layout styles and background attributes", () => {
    const rendered = buildRenderableHtml(
      '<table background="images/panel.png" style="background-color: #f7f7f7; color: #222; padding: 24px; width: 640px;"><tbody><tr><td>body</td></tr></tbody></table>',
      []
    );
    const document = new DOMParser().parseFromString(rendered, "text/html");
    const table = document.querySelector("table");
    const inlineStyle = table?.getAttribute("style") ?? "";

    expect(table?.getAttribute("background")).toBe("images/panel.png");
    expect(inlineStyle).toContain("background-color: #f7f7f7;");
    expect(inlineStyle).toContain("color: #222;");
    expect(inlineStyle).toContain("padding: 24px;");
    expect(inlineStyle).toContain("width: 640px;");
  });

  it("removes inline SVG and namespaced href attributes inside it", () => {
    const rendered = buildRenderableHtml(
      '<p>before</p><svg><a xlink:href="javascript:alert(1)"><text>bad</text></a></svg><p>after</p>',
      []
    );
    const document = new DOMParser().parseFromString(rendered, "text/html");

    expect(document.querySelector("svg")).toBeNull();
    expect(document.querySelector("[xlink\\:href]")).toBeNull();
    expect(document.body.textContent).toContain("before");
    expect(document.body.textContent).toContain("after");
    expect(document.body.textContent).not.toContain("bad");
  });

  it("does not turn non-image CID resources into data URLs", () => {
    const rendered = buildRenderableHtml('<img src="cid:note@example.com">', [textAttachment]);
    const document = new DOMParser().parseFromString(rendered, "text/html");

    expect(document.querySelector("img")?.hasAttribute("src")).toBe(false);
  });

  it("removes javascript image sources", () => {
    const rendered = buildRenderableHtml('<img src="javascript:alert(1)">', []);
    const document = new DOMParser().parseFromString(rendered, "text/html");

    expect(document.querySelector("img")?.hasAttribute("src")).toBe(false);
  });

  it("keeps remote image sources but strips remote media sources", () => {
    const rendered = buildRenderableHtml(
      '<img src="https://cdn.example.com/logo.png"><video src="https://cdn.example.com/movie.mp4"></video><audio src="https://cdn.example.com/clip.mp3"></audio><source src="https://cdn.example.com/clip.webm">',
      []
    );
    const document = new DOMParser().parseFromString(rendered, "text/html");

    expect(document.querySelector("img")?.getAttribute("src")).toBe("https://cdn.example.com/logo.png");
    expect(document.querySelector("video")?.hasAttribute("src")).toBe(false);
    expect(document.querySelector("audio")?.hasAttribute("src")).toBe(false);
    expect(document.querySelector("source")?.hasAttribute("src")).toBe(false);
  });
});
