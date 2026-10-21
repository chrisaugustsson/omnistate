// US-042: the doc viewer's stylesheet and the document of its frame.
//
// - One shared stylesheet for HTML docs is web/src/viewer/doc.css. It styles
//   every block of the format (docs/formats/html-docs.md): headings, paragraphs,
//   quotes, dividers, lists, tables, figures with their captions, the three
//   kinds of callout, code, images, requirements and user stories. It is
//   readable (a limited line length and a generous line height) and
//   print-friendly (an `@media print` block that keeps figures, tables and
//   code from breaking across pages), and it fetches nothing: no `@import`, no
//   `url()` but `data:`. The export of US-047 uses the same file.
// - src/viewer/document.ts exports `viewerDocument({ workspaceId, origin })`:
//   the whole HTML document of the viewer frame, which the doc page gives the
//   frame as its `srcdoc`. The document is self-contained: it embeds the
//   stylesheet's text in a `<style>` and the viewer's own script inline, and
//   links nothing. Its content security policy (a meta tag before every
//   script, checked by src/testing/viewer-policy.ts) blocks all network access
//   except the attachments of the workspace, under
//   `<origin>/api/v1/w/<workspace id>/attachments/`, and lets only the
//   viewer's own scripts run. The doc's HTML is not in the document: the page
//   posts it to the frame (see doc-viewer-frame.test.ts).
import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { metaPolicies, viewerPolicyProblems } from "./testing/viewer-policy";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const ORIGIN = "https://omni.example.test";
const STYLESHEET = new URL("./viewer/doc.css", import.meta.url);

type DocumentModule = {
  viewerDocument(options: { workspaceId: string; origin: string }): string;
};

/** Imports src/viewer/document.ts, failing the test (not the suite) when it is missing. */
async function loadDocument(): Promise<DocumentModule> {
  const specifier = "./viewer/document";
  try {
    return (await import(/* @vite-ignore */ specifier)) as DocumentModule;
  } catch (error) {
    expect.fail(`src/viewer/document.ts cannot be imported: ${String(error)}`);
  }
}

/** The text of the stylesheet, failing the test when it is missing. */
function stylesheet(): string {
  try {
    return readFileSync(STYLESHEET, "utf8");
  } catch (error) {
    expect.fail(`web/src/viewer/doc.css cannot be read: ${String(error)}`);
  }
}

/** The stylesheet without its comments. */
function rules(css: string) {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

/** Every selector list of the stylesheet: the text before each `{` that opens a rule. */
function selectors(css: string) {
  return Array.from(rules(css).matchAll(/([^{}]+)\{/g), (match) => match[1].trim()).filter(
    (selector) => !selector.startsWith("@"),
  );
}

/** The body of the first `@media print` block. */
function printBlock(css: string): string | null {
  const text = rules(css);
  const start = text.search(/@media[^{]*\bprint\b[^{]*\{/);
  if (start < 0) return null;
  let depth = 0;
  for (let i = text.indexOf("{", start); i < text.length; i += 1) {
    if (text[i] === "{") depth += 1;
    if (text[i] === "}") depth -= 1;
    if (depth === 0) return text.slice(text.indexOf("{", start) + 1, i);
  }
  return null;
}

describe("the shared doc stylesheet", () => {
  test("it styles every block of the HTML doc format", () => {
    const all = selectors(stylesheet()).join("\n");
    const tags = ["h1", "h2", "h3", "h4", "h5", "h6", "p", "blockquote", "hr", "ul", "ol", "li"];
    tags.push("table", "th", "td", "figure", "figcaption", "pre", "code", "img", "aside");
    for (const tag of tags) {
      expect(all, `a rule for <${tag}>`).toMatch(new RegExp(`(^|[^\\w-])${tag}(?![\\w-])`, "m"));
    }
    for (const kind of ["note", "warning", "decision"]) {
      expect(all, `a rule for the ${kind} callout`).toMatch(
        new RegExp(`\\[\\s*data-callout\\s*=\\s*["']?${kind}["']?\\s*\\]`),
      );
    }
    for (const block of ["requirement", "user-story"]) {
      expect(all, `a rule for the ${block} block`).toMatch(
        new RegExp(`\\[\\s*data-block\\s*=\\s*["']?${block}["']?\\s*\\]`),
      );
    }
  });

  test("it is readable: a limited line length and a generous line height", () => {
    const css = rules(stylesheet());
    expect(css, "a max-width in ch, em or rem keeps lines short").toMatch(
      /max-width\s*:\s*[\d.]+(ch|em|rem)\b/,
    );
    const heights = [
      ...Array.from(css.matchAll(/line-height\s*:\s*([\d.]+)\s*(;|}|!)/g), (m) => Number(m[1])),
      ...Array.from(css.matchAll(/(?<![\w-])font\s*:[^;}]*\/\s*([\d.]+)(\s|;|})/g), (m) => Number(m[1])),
    ];
    expect(
      heights.some((height) => height >= 1.4 && height <= 2),
      `a unitless line height of 1.4 to 2 for the text, not ${JSON.stringify(heights)}`,
    ).toBe(true);
  });

  test("it is print-friendly: blocks are not broken across pages", () => {
    const print = printBlock(stylesheet());
    expect(print, "an @media print block").not.toBeNull();
    expect(print).toMatch(/(page-)?break-inside\s*:\s*avoid/);
  });

  test("it fetches nothing", () => {
    const css = rules(stylesheet());
    expect(css).not.toMatch(/@import/);
    for (const match of css.matchAll(/url\(\s*(['"]?)([^)'"]*)\1\s*\)/g)) {
      expect(match[2], "only data: URLs").toMatch(/^data:/);
    }
  });
});

describe("the viewer frame's document", () => {
  test("it embeds the shared stylesheet and links nothing", async () => {
    const { viewerDocument } = await loadDocument();
    const html = viewerDocument({ workspaceId: WORKSPACE_ID, origin: ORIGIN });
    const styles = Array.from(html.matchAll(/<style\b[^>]*>([\s\S]*?)<\/style>/gi), (m) => m[1]);
    expect(
      styles.some((style) => style.includes(stylesheet().trim())),
      "a <style> of the document holds the whole of web/src/viewer/doc.css",
    ).toBe(true);
    // The markup, with the text of the inline scripts left out.
    const markup = html.replace(/(<script\b[^>]*>)[\s\S]*?(<\/script\s*>)/gi, "$1$2");
    expect(markup).not.toMatch(/<link\b/i);
    expect(markup).not.toMatch(/<script\b[^>]*\bsrc\s*=/i);
    expect(markup).not.toMatch(/<(iframe|object|embed|base)\b/i);
  });

  test("its policy blocks the network but the workspace's attachments and runs only the viewer's script", async () => {
    const { viewerDocument } = await loadDocument();
    const html = viewerDocument({ workspaceId: WORKSPACE_ID, origin: ORIGIN });
    expect(viewerPolicyProblems(html, { workspaceId: WORKSPACE_ID, origin: ORIGIN })).toEqual([]);
  });

  test("the policy names the workspace it was built for and no other", async () => {
    const { viewerDocument } = await loadDocument();
    const other = "0192d7a0-0000-7000-8000-00000000000b";
    const html = viewerDocument({ workspaceId: other, origin: ORIGIN });
    expect(viewerPolicyProblems(html, { workspaceId: other, origin: ORIGIN })).toEqual([]);
    const [{ policy }] = metaPolicies(html);
    expect(JSON.stringify([...policy])).not.toContain(WORKSPACE_ID);
  });
});

describe("the policy check itself", () => {
  // The check must find a policy that lets other scripts or the network in;
  // these documents are what a careless viewer would build.
  const ok = `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src ${ORIGIN}/api/v1/w/${WORKSPACE_ID}/attachments/ data:; style-src 'unsafe-inline'; script-src 'sha256-HASH'"></head><body><script>run()</script></body></html>`;
  const options = { workspaceId: WORKSPACE_ID, origin: ORIGIN };

  test("it reports scripts that the policy does not allow, and network sources", () => {
    expect(viewerPolicyProblems(ok, options).join("\n")).toMatch(/not allowed by the policy/);
    const lax = ok.replace("script-src 'sha256-HASH'", "script-src 'unsafe-inline'");
    expect(viewerPolicyProblems(lax, options).join("\n")).toMatch(/'unsafe-inline'/);
    const open = ok.replace("default-src 'none'", "default-src 'self'");
    expect(viewerPolicyProblems(open, options).join("\n")).toMatch(/default-src/);
    const elsewhere = ok.replace(`${ORIGIN}/api/v1/w/${WORKSPACE_ID}/attachments/`, `${ORIGIN}/`);
    expect(viewerPolicyProblems(elsewhere, options).join("\n")).toMatch(/img-src/);
    expect(viewerPolicyProblems("<p>no policy</p>", options)).not.toEqual([]);
  });
});
