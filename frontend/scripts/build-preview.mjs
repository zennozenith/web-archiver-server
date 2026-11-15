#!/usr/bin/env node
/**
 * Build a fully static, placeholder-data copy of every page route.
 *
 * The Rust server renders these templates with Askama. This script implements
 * the small Askama/Jinja subset the templates use so the same `.html` files can
 * be rendered without a backend:
 *
 *   - {% extends %} / {% block %} / {% include %}
 *   - {% if %} / {% else %} / {% endif %}, including `if let Some(x) = ...`
 *   - {% for %} / {% else %} / {% endfor %} and one struct destructuring form
 *   - {{ ... }} expressions with HTML escaping
 *
 * Asset URLs are produced by Vite (`vp build`), then rewritten to relative
 * paths so the generated tree works when opened directly from disk.
 */

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  archivePage,
  collectionFormPage,
  collectionPage,
  collections,
  formats,
  linkDetailPage,
  links,
  notFoundPage,
  page,
  pendingPage,
  sections,
  stats,
  tagPage,
  tags,
} from "./preview-data.mjs";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const frontendDir = path.resolve(scriptDir, "..");
const outDir = path.join(frontendDir, "preview");
const viteOut = path.join(outDir, "static", "vite");
const templatesRoot = path.join(viteOut, "src", "templates");

// Captured blobs only exist on the server. Point those API links at the
// matching link page so the static preview has no dead ends.
const capturePageIds = new Map(
  archivePage().pages.flatMap((entry) => entry.captures.map((capture) => [capture.id, entry.id])),
);
const linkIds = new Set(links.map((entry) => entry.id));

// The templates use a few Rust-style collection/Option helpers. Arrays get
// the same methods here; Options are plain objects from preview-data.mjs.
for (const [name, fn] of Object.entries({
  is_empty() {
    return this.length === 0;
  },
  len() {
    return this.length;
  },
  to_string() {
    return this.join(", ");
  },
})) {
  Object.defineProperty(Array.prototype, name, {
    value: fn,
    configurable: true,
    enumerable: false,
  });
}

Object.defineProperty(String.prototype, "to_string", {
  value: function to_string() {
    return this.toString();
  },
  configurable: true,
  enumerable: false,
});

// ───────────────────────────────────────────────────────────────────────────
// Tiny Askama-compatible renderer
// ───────────────────────────────────────────────────────────────────────────

const escapeHtml = (value) =>
  String(value).replace(
    /[&<>"']/g,
    (character) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[character],
  );

function tokenize(source, file) {
  const tokens = [];
  let cursor = 0;

  while (cursor < source.length) {
    const start = source.indexOf("{", cursor);
    if (start === -1) {
      tokens.push({ type: "text", value: source.slice(cursor) });
      break;
    }
    if (start > cursor) {
      tokens.push({ type: "text", value: source.slice(cursor, start) });
    }

    const marker = source[start + 1];
    if (marker === "#") {
      const end = source.indexOf("#}", start + 2);
      if (end === -1) throw new Error(`Unclosed comment in ${file}`);
      cursor = end + 2;
      continue;
    }
    if (marker === "{") {
      const end = source.indexOf("}}", start + 2);
      if (end === -1) throw new Error(`Unclosed expression in ${file}`);
      tokens.push({ type: "expr", value: source.slice(start + 2, end).trim() });
      cursor = end + 2;
      continue;
    }
    if (marker === "%") {
      const end = source.indexOf("%}", start + 2);
      if (end === -1) throw new Error(`Unclosed tag in ${file}`);
      tokens.push({ type: "tag", value: source.slice(start + 2, end).trim() });
      cursor = end + 2;
      continue;
    }

    // A normal `{` (Alpine expression, inline JS, etc.).
    tokens.push({ type: "text", value: "{" });
    cursor = start + 1;
  }

  return tokens;
}

function unquote(value) {
  const text = value.trim();
  if ((text.startsWith('"') && text.endsWith('"')) || (text.startsWith("'") && text.endsWith("'"))) {
    return text.slice(1, -1);
  }
  return text;
}

function parseTemplateSource(source, file) {
  const tokens = tokenize(source, file);
  let cursor = 0;

  function parseNodes(stopHeads) {
    const nodes = [];

    while (cursor < tokens.length) {
      const token = tokens[cursor];

      if (token.type === "text") {
        nodes.push(token);
        cursor += 1;
        continue;
      }

      if (token.type === "expr") {
        nodes.push(token);
        cursor += 1;
        continue;
      }

      const tag = token.value;
      const head = tag.split(/\s+/, 1)[0];

      if (stopHeads.includes(head)) {
        return { nodes, stop: tag };
      }

      if (head === "extends") {
        nodes.push({ type: "extends", path: unquote(tag.slice("extends".length)) });
        cursor += 1;
        continue;
      }

      if (head === "include") {
        nodes.push({ type: "include", path: unquote(tag.slice("include".length)) });
        cursor += 1;
        continue;
      }

      if (head === "block") {
        const name = tag.slice("block".length).trim();
        cursor += 1;
        const body = parseNodes(["endblock"]);
        if (!body.stop) throw new Error(`Unclosed block ${name} in ${file}`);
        cursor += 1;
        nodes.push({ type: "block", name, nodes: body.nodes });
        continue;
      }

      if (head === "if") {
        const condition = tag.slice("if".length).trim();
        cursor += 1;
        const body = parseNodes(["else", "endif"]);
        let elseNodes = [];

        if (body.stop?.startsWith("else")) {
          cursor += 1;
          const otherwise = parseNodes(["endif"]);
          elseNodes = otherwise.nodes;
        }
        if (!body.stop) throw new Error(`Unclosed if in ${file}`);
        cursor += 1;
        nodes.push({
          type: "if",
          condition,
          nodes: body.nodes,
          elseNodes,
        });
        continue;
      }

      if (head === "for") {
        const declaration = tag.slice("for".length).trim();
        const inIndex = declaration.indexOf(" in ");
        if (inIndex === -1) throw new Error(`Invalid for loop in ${file}: ${tag}`);

        const pattern = declaration.slice(0, inIndex).trim();
        const expression = declaration.slice(inIndex + 4).trim();

        cursor += 1;
        const body = parseNodes(["else", "endfor"]);
        let elseNodes = [];

        if (body.stop?.startsWith("else")) {
          cursor += 1;
          const otherwise = parseNodes(["endfor"]);
          elseNodes = otherwise.nodes;
        }
        if (!body.stop) throw new Error(`Unclosed for in ${file}`);
        cursor += 1;
        nodes.push({
          type: "for",
          pattern,
          expression,
          nodes: body.nodes,
          elseNodes,
        });
        continue;
      }

      throw new Error(`Unsupported template tag in ${file}: {% ${tag} %}`);
    }

    return { nodes, stop: null };
  }

  return parseNodes([]).nodes;
}

const templateCache = new Map();

function loadTemplate(templatePath) {
  if (templateCache.has(templatePath)) return templateCache.get(templatePath);

  const absolute = path.join(templatesRoot, templatePath);
  if (!fs.existsSync(absolute)) {
    throw new Error(`Template not found: ${templatePath} (${absolute})`);
  }

  const source = fs.readFileSync(absolute, "utf8");
  const nodes = parseTemplateSource(source, templatePath);
  templateCache.set(templatePath, nodes);
  return nodes;
}

const expressionCache = new Map();

function evaluate(expression, scope, templatePath) {
  let fn = expressionCache.get(expression);
  if (!fn) {
    // eslint-disable-next-line no-new-func, no-implied-eval
    fn = new Function("__scope", `with (__scope) { return (${expression}); }`);
    expressionCache.set(expression, fn);
  }

  try {
    return fn(scope);
  } catch (error) {
    throw new Error(`Failed to evaluate {{ ${expression} }} in ${templatePath}: ${error.message}`, {
      cause: error,
    });
  }
}

const IF_LET_SOME = /^let\s+Some\(\s*([A-Za-z_]\w*)\s*\)\s*=\s*([\s\S]+)$/;

function evaluateCondition(condition, scope, templatePath) {
  const binding = condition.match(IF_LET_SOME);
  if (binding) {
    const value = evaluate(binding[2], scope, templatePath);
    if (value !== null && value !== undefined) {
      scope[binding[1]] = value;
      return true;
    }
    return false;
  }

  return Boolean(evaluate(condition, scope, templatePath));
}

const STRUCT_PATTERN = /^[A-Za-z_]\w*\s*\{([\s\S]*)\}$/;

function bindForPattern(target, item, pattern) {
  const struct = pattern.match(STRUCT_PATTERN);
  if (!struct) {
    target[pattern.trim()] = item;
    return;
  }

  for (const field of struct[1].split(",")) {
    const name = field.trim();
    if (!name || name === "..") continue;
    target[name] = item[name];
  }
}

function renderTemplate(templatePath, context, overrides = {}) {
  const nodes = loadTemplate(templatePath);
  const extendsNode = nodes.find((node) => node.type === "extends");

  if (extendsNode) {
    const childBlocks = {};
    for (const node of nodes) {
      if (node.type === "block") childBlocks[node.name] = node.nodes;
    }
    return renderTemplate(extendsNode.path, context, {
      ...overrides,
      ...childBlocks,
    });
  }

  return renderNodes(nodes, context, overrides, templatePath);
}

function renderNodes(nodes, scope, overrides, templatePath) {
  let output = "";

  for (const node of nodes) {
    if (node.type === "text") {
      output += node.value;
      continue;
    }

    if (node.type === "expr") {
      const value = evaluate(node.value, scope, templatePath);
      output += value === null || value === undefined ? "" : escapeHtml(value);
      continue;
    }

    if (node.type === "include") {
      output += renderTemplate(node.path, scope, overrides);
      continue;
    }

    if (node.type === "block") {
      const override = overrides[node.name];
      output += renderNodes(override ?? node.nodes, scope, overrides, templatePath);
      continue;
    }

    if (node.type === "if") {
      if (evaluateCondition(node.condition, scope, templatePath)) {
        output += renderNodes(node.nodes, scope, overrides, templatePath);
      } else {
        output += renderNodes(node.elseNodes, scope, overrides, templatePath);
      }
      continue;
    }

    if (node.type === "for") {
      const iterable = evaluate(node.expression, scope, templatePath);
      const items = iterable === null || iterable === undefined ? [] : Array.from(iterable);

      if (items.length === 0) {
        output += renderNodes(node.elseNodes, scope, overrides, templatePath);
        continue;
      }

      for (const item of items) {
        const childScope = Object.create(scope);
        bindForPattern(childScope, item, node.pattern);
        output += renderNodes(node.nodes, childScope, overrides, templatePath);
      }
      continue;
    }

    if (node.type === "extends") {
      continue;
    }

    throw new Error(`Unknown node ${node.type} in ${templatePath}`);
  }

  return output;
}

// ───────────────────────────────────────────────────────────────────────────
// Static route mapping
// ───────────────────────────────────────────────────────────────────────────

function routeToFile(url) {
  const parsed = new URL(url, "http://preview.local");
  let pathname = parsed.pathname;

  if (pathname === "/") return "index.html";

  if (pathname === "/pending" && parsed.searchParams.get("status")) {
    pathname = `/pending/${parsed.searchParams.get("status").replaceAll("_", "-")}`;
  } else if (/^\/links\/[^/]+$/.test(pathname) && parsed.searchParams.has("edit")) {
    pathname = `${pathname}/edit`;
  }

  pathname = pathname.replace(/\/+$/, "");
  return `${pathname.replace(/^\/+/, "")}/index.html`;
}

function rewriteHtml(html, outputFile) {
  const outputDir = path.posix.dirname(outputFile);
  const relative = (target) => path.posix.relative(outputDir, target);

  // Vite emits `/static/vite/...` because the server mounts the bundle there.
  html = html.replace(
    /\b(href|src)="\/static\/vite\/([^"]*)"/g,
    (_match, attribute, rest) => `${attribute}="${relative(path.posix.join("static", "vite", rest))}"`,
  );

  // Root-relative navigation links become links to the generated route files.
  // API and asset URLs are intentionally left alone.
  html = html.replace(
    /\bhref="(\/(?!\/|static\/|api\/)[^"]*)"/g,
    (_match, url) => `href="${relative(routeToFile(url))}"`,
  );

  // The archive dashboard links to capture/page blobs under /api. Those are not
  // part of the static preview, so send readers to the matching link instead.
  html = html.replace(/\bhref="(\/api\/[^"]*)"/g, (match, url) => {
    let target = url.match(/^\/api\/page\/(\d+)\//);
    if (target) {
      const pageId = Number(target[1]);
      if (linkIds.has(pageId)) {
        return `href="${relative(routeToFile(`/links/${pageId}`))}"`;
      }
      return match;
    }

    target = url.match(/^\/api\/capture\/(\d+)\//);
    if (target) {
      const pageId = capturePageIds.get(Number(target[1]));
      if (pageId !== undefined && linkIds.has(pageId)) {
        return `href="${relative(routeToFile(`/links/${pageId}`))}"`;
      }
    }

    return match;
  });

  // Without Alpine (notably when opened over file://), x-cloak elements would
  // be visible. Alpine normally supplies this rule itself.
  html = html.replace("</head>", "  <style>[x-cloak]{display:none!important}</style>\n</head>");

  return html;
}

// ───────────────────────────────────────────────────────────────────────────
// Build + generate
// ───────────────────────────────────────────────────────────────────────────

function buildViteBundle() {
  const localVp = path.join(frontendDir, "node_modules", ".bin", "vp");
  const command = fs.existsSync(localVp) ? localVp : "vp";

  fs.rmSync(outDir, { recursive: true, force: true });

  execFileSync(command, ["build"], {
    cwd: frontendDir,
    stdio: "inherit",
    env: {
      ...process.env,
      OUT_DIR: path.relative(frontendDir, viteOut),
    },
  });

  if (!fs.existsSync(templatesRoot)) {
    throw new Error(
      `Vite did not emit the template tree at ${templatesRoot}. Is the frontend build configured correctly?`,
    );
  }
}

function outputPath(route) {
  return routeToFile(route);
}

const pages = [];

// Root and dashboard.
pages.push({
  output: "index.html",
  template: "library/dashboard.html",
  title: "dashboard",
  context: () => ({ page: page("dashboard"), stats, sections }),
});
pages.push({
  output: outputPath("/dashboard"),
  template: "library/dashboard.html",
  title: "dashboard",
  context: () => ({ page: page("dashboard"), stats, sections }),
});

// Library.
pages.push({
  output: outputPath("/links"),
  template: "library/links.html",
  title: "links",
  context: () => ({ page: page("links"), links, next_cursor: null }),
});

for (const entry of links) {
  pages.push({
    output: outputPath(`/links/${entry.id}`),
    template: "library/link_detail.html",
    title: `link ${entry.id}`,
    context: () => linkDetailPage(entry.id),
  });

  pages.push({
    output: outputPath(`/links/${entry.id}?edit=1`),
    template: "library/link_detail.html",
    title: `edit link ${entry.id}`,
    context: () => linkDetailPage(entry.id, true),
  });

  const archiveFormats = [formats.readable, formats.webpage, formats.screenshot];
  for (const archiveFormat of archiveFormats) {
    pages.push({
      output: outputPath(`/links/${entry.id}/archive/${archiveFormat.key}`),
      template: "library/link_archive.html",
      title: `link ${entry.id} ${archiveFormat.key}`,
      context: () => ({
        page: page("links"),
        link: entry,
        format: archiveFormat,
      }),
    });
  }
}

pages.push({
  output: outputPath("/pinned"),
  template: "library/pinned.html",
  title: "pinned",
  context: () => ({
    page: page("pinned"),
    links: links.filter((entry) => entry.pinned),
  }),
});

// Collections.
pages.push({
  output: outputPath("/collections"),
  template: "library/collections.html",
  title: "collections",
  context: () => ({ page: page("collections"), collections }),
});

pages.push({
  output: outputPath("/collections/new"),
  template: "library/collection_form.html",
  title: "new collection",
  context: () => collectionFormPage(false),
});

pages.push({
  output: outputPath("/collections/unorganized"),
  template: "library/unorganized.html",
  title: "unorganized",
  context: () => ({
    page: page("unorganized"),
    links: links.slice(0, 2),
  }),
});

for (const entry of collections) {
  pages.push({
    output: outputPath(`/collections/${entry.id}`),
    template: "library/collection.html",
    title: `collection ${entry.id}`,
    context: () => collectionPage(entry.id),
  });

  pages.push({
    output: outputPath(`/collections/${entry.id}/edit`),
    template: "library/collection_form.html",
    title: `edit collection ${entry.id}`,
    context: () => collectionFormPage(true, entry.id),
  });

  if (entry.public) {
    pages.push({
      output: outputPath(`/public/collections/${entry.id}`),
      template: "library/public_collection.html",
      title: `public collection ${entry.id}`,
      context: () => ({
        page: page("collections"),
        collection: entry,
        links: links.filter((link) => link.collection_id === entry.id),
      }),
    });
  }
}

// Pending queue, including each filter tab.
pages.push({
  output: outputPath("/pending"),
  template: "library/pending.html",
  title: "pending",
  context: () => pendingPage(),
});

for (const [status, label] of [
  ["queued", "queued"],
  ["in_progress", "in-progress"],
  ["failed", "failed"],
]) {
  pages.push({
    output: outputPath(`/pending?status=${status}`),
    template: "library/pending.html",
    title: `pending ${label}`,
    context: () => pendingPage(status),
  });
}

// Tags.
pages.push({
  output: outputPath("/tags"),
  template: "library/tags.html",
  title: "tags",
  context: () => ({ page: page("tags"), tags }),
});

for (const entry of tags) {
  pages.push({
    output: outputPath(`/tags/${entry.slug}`),
    template: "library/tag.html",
    title: `tag ${entry.slug}`,
    context: () => tagPage(entry.name),
  });
}

// Archive dashboard.
pages.push({
  output: outputPath("/archive"),
  template: "archive/index.html",
  title: "archive",
  context: archivePage,
});

pages.push({
  output: outputPath("/archive/counter"),
  template: "_vite-counter.html",
  title: "counter",
  context: () => ({ title: "Rust + Axum + Vite + Alpine" }),
});

// A rendered 404 page, handy for checking the error layout.
pages.push({
  output: "404/index.html",
  template: "error.html",
  title: "not found",
  context: notFoundPage,
});

function main() {
  buildViteBundle();

  for (const entry of pages) {
    const html = rewriteHtml(renderTemplate(entry.template, entry.context()), entry.output);

    const destination = path.join(outDir, entry.output);
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.writeFileSync(destination, html);
  }

  // The template cache and manifest are build artifacts; only the pages and
  // static assets are useful to someone opening the preview.
  fs.rmSync(path.join(viteOut, "src"), { recursive: true, force: true });
  fs.rmSync(path.join(viteOut, ".vite"), { recursive: true, force: true });

  console.log(`\nStatic preview written to ${path.relative(process.cwd(), outDir)} (${pages.length} pages).`);
  console.log(`Open ${path.join(outDir, "index.html")} in a browser.`);
}

main();
