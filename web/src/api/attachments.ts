const PREFIX = "attachment:";

/** The ID in `attachment:<id>`, or `null` for any other text. */
export function attachmentId(reference: string): string | null {
  return reference.startsWith(PREFIX) && reference.length > PREFIX.length ? reference.slice(PREFIX.length) : null;
}

/** The bytes of a workspace attachment, an image or a font file. Throws when the API does not serve them. */
export async function loadAttachment(workspaceId: string, attachment: string): Promise<Uint8Array> {
  const response = await fetch(`/api/v1/w/${workspaceId}/attachments/${attachment}`, { credentials: "same-origin" });
  if (!response.ok) throw new Error(`the API answered ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}
