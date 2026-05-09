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

  it("removes style elements and strips inline style attributes", () => {
    const rendered = buildRenderableHtml(
      '<style>@import url("https://tracker.example.com/mail.css");</style><p style="background-image:url(https://tracker.example.com/pixel)">body</p>',
      []
    );
    const document = new DOMParser().parseFromString(rendered, "text/html");

    expect(document.querySelector("style")).toBeNull();
    expect(document.querySelector("p")?.hasAttribute("style")).toBe(false);
    expect(document.body.textContent).toContain("body");
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
