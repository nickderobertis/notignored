// Render a `--format markdown` comment body to the two PNGs the README embeds.
//
// Input is the body on stdin — the real binary's real output, produced by
// scripts/pr-comment-png.sh — and output is a light/dark pair of screenshots of
// that body laid out the way GitHub lays a comment out: markdown-it for the
// HTML, Shiki for the syntax colours inside the fenced blocks, comment.css for
// the Primer styling, and a headless Chromium for the raster.
//
// Nothing here invents content. The only thing this script decides that the
// binary did not is which `<details>` is open — the first one, so the picture
// shows a reviewer what a suppressed snippet looks like — which is a click a
// reader makes for themselves.
//
// Usage: node scripts/comment-render/render.mjs <light.png> <dark.png> < body.md

import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..", "..");
const dev = join(root, ".dev", "comment-render");

// The pinned toolchain lives in .dev/, not beside this file, so resolve every
// dependency from there rather than from this script's own directory.
const fromDev = createRequire(join(dev, "package.json"));
let MarkdownIt;
let chromium;
let shiki;
try {
  MarkdownIt = fromDev("markdown-it");
  ({ chromium } = fromDev("playwright"));
  shiki = await import(pathToFileURL(fromDev.resolve("shiki")).href);
} catch (error) {
  process.stderr.write(
    `render.mjs: the pinned render toolchain is not installed in ${dev} (${error.message})\n` +
      "ACTION: run 'just screenshots-comment-tools'\n",
  );
  process.exit(1);
}

const [lightOut, darkOut] = process.argv.slice(2);
if (!lightOut || !darkOut) {
  process.stderr.write(
    "render.mjs: usage: node scripts/comment-render/render.mjs <light.png> <dark.png> < body.md\n" +
      "ACTION: run 'just screenshots-pr-comment', which supplies both paths\n",
  );
  process.exit(1);
}

const body = readFileSync(0, "utf8");
if (!body.includes("<!-- notignored-report -->")) {
  process.stderr.write(
    "render.mjs: stdin does not look like a rendered comment body — no notignored marker\n" +
      "ACTION: check that `notignored --diff --format markdown` still emits the sticky marker\n",
  );
  process.exit(1);
}

// Shiki tokenizes; GitHub's own `pl-*` class names carry the colour. Mapping
// the two lets one document serve both themes — comment.css holds the light and
// dark value of every class — and keeps the markup the same as the markup a real
// comment serves. The keys are `github-light`'s token colours, which are the
// palette GitHub shipped before Primer's 2022 refresh; comment.css restates each
// one at the value a comment renders today (sampled from a real capture).
const PL_CLASS = new Map([
  ["#6A737D", "pl-c"], // comment
  ["#586069", "pl-c"], // punctuation, unmatched brackets
  ["#005CC5", "pl-c1"], // constant
  ["#22863A", "pl-ent"], // tag
  ["#6F42C1", "pl-en"], // entity
  ["#24292E", "pl-smi"], // identifier
  ["#D73A49", "pl-k"], // keyword
  ["#032F62", "pl-s"], // string
  ["#E36209", "pl-v"], // variable
  ["#B31D28", "pl-bu"], // invalid
]);

// The languages the renderer can name in a fence. `src/cli/markdown.rs` derives
// each from `crate::source::Language`, so this list is the same closed set, and
// `tests/screenshots_contract.rs` fails the build when the two stop agreeing.
const LANGUAGES = [
  "python",
  "rust",
  "javascript",
  "typescript",
  "bash",
  "yaml",
  "toml",
];

// One snippet line: the gutter `src/cli/markdown.rs::snippet` writes — a `>` on
// the suppressed line, the 1-based line number, ` | ` — and then the source.
const SNIPPET_LINE = /^([ >]*)(\d+) \|(?: (.*))?$/;

const highlighter = await shiki.createHighlighter({
  themes: ["github-light"],
  langs: LANGUAGES,
});

const escapeHtml = (text) =>
  text.replace(
    /[&<>]/g,
    (character) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[character],
  );

// GitHub's own copy-to-clipboard control, which sits inside every fenced block
// in a rendered comment. Octicon `copy`, 16px, as the real page serves it.
const CLIPBOARD_BUTTON = `<div class="zeroclipboard-container"><button class="ClipboardButton btn" aria-label="Copy code to clipboard"><svg aria-hidden="true" height="16" viewBox="0 0 16 16" width="16" class="octicon octicon-copy"><path d="M0 6.75C0 5.784.784 5 1.75 5h1.5a.75.75 0 0 1 0 1.5h-1.5a.25.25 0 0 0-.25.25v7.5c0 .138.112.25.25.25h7.5a.25.25 0 0 0 .25-.25v-1.5a.75.75 0 0 1 1.5 0v1.5A1.75 1.75 0 0 1 9.25 16h-7.5A1.75 1.75 0 0 1 0 14.25Z"></path><path d="M5 1.75C5 .784 5.784 0 6.75 0h7.5C15.216 0 16 .784 16 1.75v7.5A1.75 1.75 0 0 1 14.25 11h-7.5A1.75 1.75 0 0 1 5 9.25Zm1.75-.25a.25.25 0 0 0-.25.25v7.5c0 .138.112.25.25.25h7.5a.25.25 0 0 0 .25-.25v-7.5a.25.25 0 0 0-.25-.25Z"></path></svg></button></div>`;

/**
 * Split a snippet into its gutters and the source they annotate.
 *
 * The gutter is notignored's, not the file's, so it must not reach the
 * highlighter: a TextMate grammar reads `> 3 |` as a number and two operators
 * and paints them, where GitHub — which highlights the same text — leaves the
 * pipes alone and marks the number a constant. Returns null when any line is
 * not a snippet line, so an unrecognized block still renders as plain source.
 */
function splitGutters(source) {
  const rows = [];
  for (const line of source.split("\n")) {
    const match = SNIPPET_LINE.exec(line);
    if (!match) {
      return null;
    }
    rows.push({ lead: match[1], number: match[2], code: match[3] ?? null });
  }
  return rows;
}

/** The `pl-*` span GitHub would wrap a token of this colour in. */
function span(token) {
  const text = escapeHtml(token.content);
  const colour = (token.color ?? "").toUpperCase();
  if (!colour) {
    return text;
  }
  const className = PL_CLASS.get(colour);
  if (!className) {
    process.stderr.write(
      `render.mjs: no GitHub syntax class for the colour ${colour} shiki gave ${JSON.stringify(token.content)}\n` +
        "ACTION: add it to PL_CLASS in this file and give it a light and a dark value in comment.css\n",
    );
    process.exit(1);
  }
  return `<span class="${className}">${text}</span>`;
}

/** Highlight one fenced block the way a rendered comment shows it. */
function highlight(source, lang) {
  const rows = splitGutters(source);
  const code = rows ? rows.map((row) => row.code ?? "").join("\n") : source;
  const { tokens } = highlighter.codeToTokens(code, {
    lang,
    theme: "github-light",
  });
  const lines = tokens.map((line) => line.map(span).join(""));
  if (!rows) {
    return lines.join("\n");
  }
  return rows
    .map((row, index) => {
      const gutter = `${escapeHtml(row.lead)}<span class="pl-c1">${row.number}</span> |`;
      return row.code === null ? gutter : `${gutter} ${lines[index]}`;
    })
    .join("\n");
}

const md = new MarkdownIt({ html: true, linkify: false, typographer: false });

// The fence rule, rather than markdown-it's `highlight` option: GitHub wraps a
// fenced block in its own `.highlight` container with the copy button inside it,
// and the option's output is only ever placed inside a `<pre><code>`.
md.renderer.rules.fence = (tokens, index) => {
  const lang = tokens[index].info.trim();
  const known = LANGUAGES.includes(lang);
  const code = highlight(
    tokens[index].content.replace(/\n$/, ""),
    known ? lang : "text",
  );
  const container = known ? `highlight highlight-source-${lang}` : "highlight";
  return `<div class="${container} notranslate position-relative overflow-auto"><pre class="notranslate">${code}</pre>${CLIPBOARD_BUTTON}</div>\n`;
};

// The first snippet open, the rest as the reviewer first meets them. A reader
// deciding whether to add the Action needs to see what is behind one of these.
const rendered = md.render(body).replace("<details>", "<details open>");
if (!rendered.includes("<details open>")) {
  process.stderr.write(
    "render.mjs: the body carries no <details> block to open\n" +
      "ACTION: check that the fixture change still adds a suppression whose code the renderer snippets\n",
  );
  process.exit(1);
}

const css = readFileSync(join(here, "comment.css"), "utf8");
const page_html = `<!doctype html>
<html lang="en" data-theme="light"><head><meta charset="utf-8"><style>${css}</style></head>
<body><div class="timeline-comment"><div class="comment-body markdown-body">
${rendered}
</div></div></body></html>`;

const browser = await chromium.launch();
try {
  // 2×, the density the reference captures were taken at and the one that keeps
  // the README embed crisp on a HiDPI screen at GitHub's ~830px content width.
  const page = await browser.newPage({
    viewport: { width: 900, height: 1200 },
    deviceScaleFactor: 2,
  });
  await page.setContent(page_html, { waitUntil: "load" });
  await page.evaluate(() => document.fonts.ready);
  const card = await page.$(".timeline-comment");
  for (const [theme, out] of [
    ["light", lightOut],
    ["dark", darkOut],
  ]) {
    await page.evaluate(
      (value) => document.documentElement.setAttribute("data-theme", value),
      theme,
    );
    await page.emulateMedia({ colorScheme: theme });
    await card.screenshot({ path: out });
  }
} finally {
  await browser.close();
}
