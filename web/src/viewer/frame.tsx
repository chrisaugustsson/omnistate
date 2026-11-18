import { useEffect, useMemo, useRef, useState } from "react";
import { viewerDocument } from "./document";
import { attachmentIds, loadAttachmentImage, withAttachmentImages } from "./html";

/** The sandboxed frame that shows the doc, and the messages between it and the page. */
export function ViewerFrame({
  workspaceId,
  html,
  pins,
  selected,
  reveal,
  onSelect,
  className = "h-[75vh] w-full rounded-md border bg-white",
  title = "Doc",
  fit = false,
}: {
  workspaceId: string;
  html: string | null;
  pins: { node: string; count: number }[];
  selected: string | null;
  /** A block that the frame scrolls into view and keeps in place once it has; left out, none is. */
  reveal?: string | null;
  onSelect: (node: string) => void;
  /** The frame's class names; the doc viewer's tall frame at first. */
  className?: string;
  title?: string;
  /** Sized to the doc, so that it reads as page text and the page scrolls; a block to reveal scrolls the page. */
  fit?: boolean;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [ready, setReady] = useState(0);
  const [height, setHeight] = useState<number | null>(null);
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
      } else if (message?.type === "viewer.size" && typeof (message as { height?: unknown }).height === "number") {
        setHeight((message as { height: number }).height);
      } else if (message?.type === "viewer.reveal" && typeof (message as { top?: unknown }).top === "number") {
        revealInPage(frame.current, (message as { top: number }).top);
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  useEffect(() => {
    if (shown === null || !ready) return;
    // The frame's origin is opaque, so "*" is the only target a page can name.
    const message = { type: "viewer.show", html: shown, pins, selected, ...(reveal ? { reveal } : {}), ...(fit ? { fit } : {}) };
    frame.current?.contentWindow?.postMessage(message, "*");
  }, [shown, pins, selected, reveal, ready, fit]);

  return (
    <iframe
      ref={frame}
      title={title}
      sandbox="allow-scripts"
      srcDoc={srcDoc}
      className={className}
      // The frame's `prefers-color-scheme` follows this element's color scheme.
      // The app is light, so a doc's dark styles must not match on a dark system.
      style={{ colorScheme: "light", ...(fit ? { height: height ?? 0, minHeight: height === null ? "12rem" : undefined } : {}) }}
    />
  );
}

/** Scrolls the page that holds a fitted `frame` so that the point `top` of the doc is in the middle of the view. */
function revealInPage(frame: HTMLIFrameElement | null, top: number) {
  if (!frame) return;
  let scroller: HTMLElement | null = frame.parentElement;
  while (scroller && !(/(auto|scroll)/.test(getComputedStyle(scroller).overflowY) && scroller.scrollHeight > scroller.clientHeight)) {
    scroller = scroller.parentElement;
  }
  if (!scroller) return;
  const offset = frame.getBoundingClientRect().top - scroller.getBoundingClientRect().top + top;
  scroller.scrollTo({ top: Math.max(0, scroller.scrollTop + offset - scroller.clientHeight / 3) });
}

