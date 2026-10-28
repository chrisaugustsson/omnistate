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
//   `<origin>/api/v1/w/<workspace id>/attachments/`. Since US-109 it may let
//   inline scripts run (a doc carries its own scripts and handlers), but no
//   script from the network. The doc's HTML is not in the document: the page
//   posts it to the frame (see doc-viewer-frame.test.ts).
import { describe, expect, test } from "vitest";
import { viewerPolicyProblems } from "./testing/viewer-policy";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const ORIGIN = "https://omni.example.test";

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

describe("the viewer frame's document", () => {
  test("its policy blocks the network but the workspace's attachments and loads no script from the network", async () => {
    const { viewerDocument } = await loadDocument();
    const html = viewerDocument({ workspaceId: WORKSPACE_ID, origin: ORIGIN });
    expect(viewerPolicyProblems(html, { workspaceId: WORKSPACE_ID, origin: ORIGIN })).toEqual([]);
  });
});
