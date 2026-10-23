// Reading a doc's printed HTML (docs/formats/html-docs.md) in the page, which
// shows it in the sandboxed frame and never inserts it into its own document.

function parse(html: string): Document {
  return new DOMParser().parseFromString(html, "text/html");
}

/** The node IDs of the blocks of the doc. */
export function blockIds(html: string): Set<string> {
  const ids = new Set<string>();
  for (const block of Array.from(parse(html).querySelectorAll("[data-omni-id]"))) {
    ids.add(block.getAttribute("data-omni-id") ?? "");
  }
  return ids;
}

/** The text of the block `node`, with its white space collapsed; `null` when the doc has no such block. */
export function blockText(html: string, node: string): string | null {
  for (const block of Array.from(parse(html).querySelectorAll("[data-omni-id]"))) {
    if (block.getAttribute("data-omni-id") === node) return (block.textContent ?? "").replace(/\s+/g, " ").trim();
  }
  return null;
}

const ATTACHMENT_SOURCE = /\bsrc="attachment:([^"]+)"/g;

/** The IDs of the attachments that images of the doc show. */
export function attachmentIds(html: string): string[] {
  return Array.from(new Set(Array.from(html.matchAll(ATTACHMENT_SOURCE), (match) => match[1])));
}

/** The doc's HTML with the images whose bytes are in `images` (by attachment ID, as `data:` URLs) shown by them. */
export function withAttachmentImages(html: string, images: Record<string, string>): string {
  return html.replace(ATTACHMENT_SOURCE, (whole, id: string) => (images[id] ? `src="${images[id]}"` : whole));
}

/** The bytes of an attachment of the workspace as a `data:` URL, read with the reviewer's session. */
export async function loadAttachmentImage(workspaceId: string, id: string): Promise<string> {
  const response = await fetch(`/api/v1/w/${workspaceId}/attachments/${encodeURIComponent(id)}`, {
    credentials: "same-origin",
  });
  if (!response.ok) throw new Error(`attachment ${id}: ${response.status}`);
  const blob = await response.blob();
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}
