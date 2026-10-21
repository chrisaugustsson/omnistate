import mermaidScript from "mermaid/dist/mermaid.min.js?raw";
import docStyles from "./doc.css?raw";
import { FRAME_SCRIPT } from "./frame-script";
import { sha256Source } from "./sha256";

// The document of the doc viewer's frame, which the doc page gives the frame
// as its `srcdoc`. The frame is `<iframe sandbox="allow-scripts">` without
// `allow-same-origin`, so its origin is opaque: it cannot read the page and
// its requests carry no session cookie. The document adds a content security
// policy that blocks all network access but the images of the workspace's
// attachments, and runs nothing but the viewer's own two scripts (Mermaid and
// the viewer's), each allowed by its hash. The doc's HTML is not part of the
// document: the page posts it to the frame (see frame-script.ts).

const VIEWER_STYLES = `
html, body { margin: 0; background: #fff; }
body { position: relative; padding: 1.5rem 3.5rem 4rem; }
.omni-doc [data-node-id] { cursor: pointer; }
.omni-doc [data-node-id].is-hover {
  outline: 2px solid rgba(9, 105, 218, 0.4);
  outline-offset: 2px;
  background-image: linear-gradient(rgba(9, 105, 218, 0.07), rgba(9, 105, 218, 0.07));
}
.omni-doc [data-node-id].is-claimed {
  outline: 2px dashed #9a6700;
  outline-offset: 2px;
}
.omni-doc [data-node-id].is-selected {
  outline: 2px solid #0969da;
  outline-offset: 2px;
  background-image: linear-gradient(rgba(9, 105, 218, 0.14), rgba(9, 105, 218, 0.14));
}
.omni-mermaid svg { max-width: 100%; height: auto; }
#pins { position: absolute; top: 0; left: 0; width: 0; height: 0; }
.omni-pin {
  position: absolute;
  min-width: 1.5rem;
  height: 1.5rem;
  padding: 0 0.35rem;
  border: 0;
  border-radius: 999px;
  background: #0969da;
  color: #fff;
  font: 600 0.75rem/1.5rem ui-sans-serif, system-ui, sans-serif;
  text-align: center;
  cursor: pointer;
}
`;

/** The hash of each script text, so that the large Mermaid script is only hashed once. */
const hashes = new Map<string, string>();

function hashOf(text: string): string {
  let hash = hashes.get(text);
  if (!hash) {
    hash = sha256Source(text);
    hashes.set(text, hash);
  }
  return hash;
}

/** A value as a JavaScript literal that cannot end its script element. */
function literal(value: string): string {
  return JSON.stringify(value).replace(/</g, "\\u003c");
}

/**
 * The viewer frame's HTML document for a doc of workspace `workspaceId`, shown
 * by the app at `origin`. Its policy lets the frame show images of that
 * workspace's attachments (`<origin>/api/v1/w/<workspace id>/attachments/`)
 * and `data:` and `blob:` images, and nothing else leave or enter the frame.
 */
export function viewerDocument({ workspaceId, origin }: { workspaceId: string; origin: string }): string {
  const viewerScript = `var PARENT_ORIGIN = ${literal(origin)};\n${FRAME_SCRIPT}`;
  const policy = [
    "default-src 'none'",
    `img-src data: blob: ${origin}/api/v1/w/${workspaceId}/attachments/`,
    "style-src 'unsafe-inline'",
    `script-src ${hashOf(mermaidScript)} ${hashOf(viewerScript)}`,
    "base-uri 'none'",
    "form-action 'none'",
  ].join("; ");
  return [
    "<!doctype html>",
    '<html lang="en">',
    "<head>",
    '<meta charset="utf-8">',
    `<meta http-equiv="Content-Security-Policy" content="${policy}">`,
    '<meta name="viewport" content="width=device-width, initial-scale=1">',
    `<style>${docStyles.trim()}</style>`,
    `<style>${VIEWER_STYLES}</style>`,
    "</head>",
    "<body>",
    '<main id="doc" class="omni-doc"></main>',
    '<div id="pins"></div>',
    `<script>${mermaidScript}</script>`,
    `<script>${viewerScript}</script>`,
    "</body>",
    "</html>",
  ].join("\n");
}
