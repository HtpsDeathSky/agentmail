import { readFileSync, statSync } from "node:fs";

const requiredFiles = [
  "docs/pages/index.html",
  "docs/pages/styles.css",
  "docs/pages/assets/icon.png",
  "docs/pages/favicon.ico",
  ".github/workflows/pages.yml"
];

for (const file of requiredFiles) {
  const stat = statSync(file);
  if (!stat.isFile()) {
    throw new Error(`${file} is not a file`);
  }
}

const html = readFileSync("docs/pages/index.html", "utf8");
const css = readFileSync("docs/pages/styles.css", "utf8");
const workflow = readFileSync(".github/workflows/pages.yml", "utf8");

const checks = [
  ["page title", html.includes("<title>AgentMail")],
  ["latest release download link", html.includes("https://github.com/HtpsDeathSky/agentmail/releases/latest")],
  ["product icon asset", html.includes("./assets/icon.png")],
  ["favicon link", html.includes('rel="icon" href="./assets/icon.png"')],
  ["repository link", html.includes("https://github.com/HtpsDeathSky/agentmail")],
  ["Windows positioning", html.includes("Windows-first")],
  ["manual AI positioning", html.includes("manual AI analysis")],
  ["plaintext SQLite notice", html.includes("plaintext in SQLite")],
  ["GitHub Pages upload path", workflow.includes("path: docs/pages")],
  ["GitHub Pages deploy action", workflow.includes("actions/deploy-pages")],
  ["industrial palette", css.includes("--bg: #0b0f0d")]
];

const failed = checks.filter(([, ok]) => !ok);
if (failed.length > 0) {
  for (const [name] of failed) {
    console.error(`Missing required Pages content: ${name}`);
  }
  process.exit(1);
}

console.log("GitHub Pages static site checks passed.");
