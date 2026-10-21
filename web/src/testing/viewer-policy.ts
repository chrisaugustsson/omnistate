// A check of the doc viewer frame's document (US-042), shared by the tests of
// the viewer: web/src/doc-viewer-document.test.ts reads the document that
// src/viewer/document.ts builds, and web/src/doc-viewer.test.tsx reads the
// `srcdoc` of the frame that the doc page renders.
//
// The frame's document carries its content security policy in a `<meta
// http-equiv="Content-Security-Policy">` that comes before every script. The
// policy blocks all network access except the workspace's attachments, and
// lets only the viewer's own scripts run: every script of the document is
// inline and allowed by its hash (or its nonce), and nothing else is.
import { createHash } from "node:crypto";

/** The fetch directives and the directives each one falls back to, in order (CSP Level 3). */
const FALLBACKS: Record<string, string[]> = {
  "script-src": ["script-src", "default-src"],
  "script-src-elem": ["script-src-elem", "script-src", "default-src"],
  "script-src-attr": ["script-src-attr", "script-src", "default-src"],
  "style-src": ["style-src", "default-src"],
  "style-src-elem": ["style-src-elem", "style-src", "default-src"],
  "style-src-attr": ["style-src-attr", "style-src", "default-src"],
  "img-src": ["img-src", "default-src"],
  "font-src": ["font-src", "default-src"],
  "media-src": ["media-src", "default-src"],
  "connect-src": ["connect-src", "default-src"],
  "object-src": ["object-src", "default-src"],
  "manifest-src": ["manifest-src", "default-src"],
  "frame-src": ["frame-src", "child-src", "default-src"],
  "worker-src": ["worker-src", "child-src", "script-src", "default-src"],
};

/** Sources that allow no network access: no host, no scheme that reaches one, no `'self'`. */
const LOCAL_SOURCES = new Set(["'none'", "data:", "blob:"]);

/** Script sources that allow nothing but the scripts that a hash or nonce names. */
function isScriptKeyword(source: string) {
  return (
    source === "'none'" ||
    source === "'unsafe-eval'" ||
    source === "'wasm-unsafe-eval'" ||
    source === "'report-sample'" ||
    /^'(sha256|sha384|sha512)-[A-Za-z0-9+/=_-]+'$/.test(source) ||
    /^'nonce-[A-Za-z0-9+/=_-]+'$/.test(source)
  );
}

function decodeEntities(text: string) {
  return text
    .replace(/&quot;/g, '"')
    .replace(/&#0*39;/g, "'")
    .replace(/&#x0*27;/gi, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

/** The attributes of a start tag such as `<meta http-equiv="..." content="...">`, by lower-case name. */
export function attributesOf(tag: string): Map<string, string> {
  const attributes = new Map<string, string>();
  const inner = tag.replace(/^<\s*[\w-]+/, "").replace(/\/?>$/, "");
  const pattern = /([^\s"'=<>`/]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;
  for (const match of inner.matchAll(pattern)) {
    const value = match[2] ?? match[3] ?? match[4] ?? "";
    attributes.set(match[1].toLowerCase(), decodeEntities(value));
  }
  return attributes;
}

/** The policies of the document's CSP meta tags, each by directive name, with the position of the tag. */
export function metaPolicies(html: string): { at: number; policy: Map<string, string[]> }[] {
  const found: { at: number; policy: Map<string, string[]> }[] = [];
  for (const match of html.matchAll(/<meta\b[^>]*>/gi)) {
    const attributes = attributesOf(match[0]);
    if ((attributes.get("http-equiv") ?? "").toLowerCase() !== "content-security-policy") continue;
    const policy = new Map<string, string[]>();
    for (const part of (attributes.get("content") ?? "").split(";")) {
      const [name, ...sources] = part.trim().split(/\s+/);
      if (!name) continue;
      // The first of two directives of the same name wins.
      if (!policy.has(name.toLowerCase())) policy.set(name.toLowerCase(), sources);
    }
    found.push({ at: match.index ?? 0, policy });
  }
  return found;
}

/** The sources that apply to `directive` under `policy`, with its fallbacks; `null` when nothing restricts it. */
export function effectiveSources(policy: Map<string, string[]>, directive: string): string[] | null {
  for (const name of FALLBACKS[directive] ?? [directive]) {
    const sources = policy.get(name);
    if (sources) return sources;
  }
  return null;
}

/** The scripts of the document: their attributes, their text and where they start. */
export function scriptsOf(html: string) {
  return Array.from(html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script\s*>/gi), (match) => ({
    at: match.index ?? 0,
    attributes: attributesOf(`<script ${match[1]}>`),
    text: match[2],
  }));
}

function hashSource(algorithm: "sha256" | "sha384" | "sha512", text: string) {
  return `'${algorithm}-${createHash(algorithm).update(text, "utf8").digest("base64")}'`;
}

/**
 * What is wrong with the frame document `html` of the viewer of a doc of
 * workspace `workspaceId`, served by the app at `origin`; empty when nothing is.
 */
export function viewerPolicyProblems(
  html: string,
  { workspaceId, origin }: { workspaceId: string; origin: string },
): string[] {
  const problems: string[] = [];
  const policies = metaPolicies(html);
  if (policies.length !== 1) {
    return [`the document has ${policies.length} content security policy meta tags, not one`];
  }
  const { at, policy } = policies[0];
  const scripts = scriptsOf(html);
  if (scripts.some((script) => script.at < at)) {
    problems.push("a script comes before the policy, so the policy does not cover it");
  }

  const defaults = policy.get("default-src");
  if (!defaults || defaults.length !== 1 || defaults[0] !== "'none'") {
    problems.push(`default-src is ${JSON.stringify(defaults)}, not 'none'`);
  }

  const connect = effectiveSources(policy, "connect-src") ?? ["*"];
  if (connect.some((source) => source !== "'none'")) {
    problems.push(`connect-src allows ${connect.join(" ")}: no request may leave the frame`);
  }

  for (const directive of ["font-src", "media-src", "object-src", "manifest-src", "frame-src", "worker-src"]) {
    const sources = effectiveSources(policy, directive) ?? ["*"];
    const network = sources.filter((source) => !LOCAL_SOURCES.has(source) && !isScriptKeyword(source));
    if (network.length) problems.push(`${directive} allows the network: ${network.join(" ")}`);
  }

  for (const directive of ["style-src", "style-src-elem", "style-src-attr"]) {
    const sources = effectiveSources(policy, directive) ?? ["*"];
    const network = sources.filter(
      (source) => !LOCAL_SOURCES.has(source) && source !== "'unsafe-inline'" && !isScriptKeyword(source),
    );
    if (network.length) problems.push(`${directive} allows the network: ${network.join(" ")}`);
  }

  const attachments = `${origin}/api/v1/w/${workspaceId}/attachments/`;
  const images = effectiveSources(policy, "img-src") ?? ["*"];
  for (const source of images) {
    if (LOCAL_SOURCES.has(source)) continue;
    if (source.startsWith(attachments)) continue;
    problems.push(`img-src allows ${source}, which is not this workspace's attachments (${attachments})`);
  }
  if (!images.some((source) => source !== "'none'")) {
    problems.push("img-src allows nothing, so no attachment image can be shown");
  }

  const allowed = new Set<string>();
  for (const directive of ["script-src", "script-src-elem", "script-src-attr"]) {
    const sources = effectiveSources(policy, directive) ?? ["*"];
    for (const source of sources) {
      if (isScriptKeyword(source)) allowed.add(source);
      else problems.push(`${directive} allows ${source}: only the viewer's own scripts, by hash or nonce, may run`);
    }
  }
  if (!scripts.length) problems.push("the document has no script: the viewer's own script runs inside it");
  for (const [index, script] of scripts.entries()) {
    if (script.attributes.has("src")) {
      problems.push(`script ${index + 1} is loaded from ${script.attributes.get("src")}: the viewer's scripts are inline`);
      continue;
    }
    const nonce = script.attributes.get("nonce");
    const byHash = (["sha256", "sha384", "sha512"] as const).some((algorithm) =>
      allowed.has(hashSource(algorithm, script.text)),
    );
    const byNonce = nonce !== undefined && allowed.has(`'nonce-${nonce}'`);
    if (!byHash && !byNonce) problems.push(`script ${index + 1} is not allowed by the policy, so it would not run`);
  }
  return problems;
}
