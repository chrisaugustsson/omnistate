import { useEffect, useMemo, useRef, useState } from "react";
import type { ClaimBody } from "@/api/generated";
import { viewerDocument } from "./document";
import { attachmentIds, loadAttachmentImage, withAttachmentImages } from "./html";

/** A block that a change set added, changed or removed, as the frame marks it. */
export type BlockChange = { node: string; change: "added" | "changed" | "removed" };

/** The sandboxed frame that shows the doc, and the messages between it and the page. */
export function ViewerFrame({
  workspaceId,
  html,
  pins,
  selected,
  claims,
  changes,
  reveal,
  onSelect,
  className = "h-[75vh] w-full rounded-md border bg-white",
  title = "Doc",
}: {
  workspaceId: string;
  html: string | null;
  pins: { node: string; count: number }[];
  selected: string | null;
  claims: ClaimBody[];
  /** The blocks that a change set added, changed or removed, which the frame marks; left out, none is. */
  changes?: BlockChange[];
  /** A block that the frame scrolls into view and keeps in place once it has; left out, none is. */
  reveal?: string | null;
  onSelect: (node: string) => void;
  /** The frame's class names; the doc viewer's tall frame at first. */
  className?: string;
  title?: string;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [ready, setReady] = useState(0);
  const srcDoc = useMemo(() => viewerDocument({ workspaceId, origin: window.location.origin }), [workspaceId]);
  const onSelectRef = useRef(onSelect);
  useEffect(() => {
    onSelectRef.current = onSelect;
  });

  // Images of attachments are read with the reviewer's session and given to
  // the frame as data: URLs, because the frame's own requests carry no session.
  const [images, setImages] = useState<Record<string, string>>({});
  const requested = useRef(new Set<string>());
  useEffect(() => {
    for (const id of html ? attachmentIds(html) : []) {
      if (requested.current.has(id)) continue;
      requested.current.add(id);
      loadAttachmentImage(workspaceId, id).then(
        (url) => setImages((previous) => ({ ...previous, [id]: url })),
        () => requested.current.delete(id),
      );
    }
  }, [html, workspaceId]);
  const shown = useMemo(() => (html === null ? null : withAttachmentImages(html, images)), [html, images]);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      if (!frame.current || event.source !== frame.current.contentWindow) return;
      const message = event.data as { type?: unknown; node?: unknown } | null;
      if (message?.type === "viewer.ready") setReady((count) => count + 1);
      else if (message?.type === "viewer.select" && typeof message.node === "string") {
        onSelectRef.current(message.node);
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  useEffect(() => {
    if (shown === null || !ready) return;
    // The frame's origin is opaque, so "*" is the only target a page can name.
    const message = { type: "viewer.show", html: shown, pins, selected, claims, ...(changes ? { changes } : {}), ...(reveal ? { reveal } : {}) };
    frame.current?.contentWindow?.postMessage(message, "*");
  }, [shown, pins, selected, claims, changes, reveal, ready]);

  return (
    <iframe
      ref={frame}
      title={title}
      sandbox="allow-scripts"
      srcDoc={srcDoc}
      className={className}
    />
  );
}

