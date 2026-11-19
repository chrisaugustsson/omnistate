import { useEffect, useRef, type ReactNode } from "react";

/**
 * A bottom sheet on the phone: a scrim over the page and a sheet rising from
 * the bottom with a grab handle. A tap on the scrim, Esc, or following a link
 * in it closes it; the focus goes to its first link and back where it was.
 */
export function BottomSheet({ label, onClose, children }: { label: string; onClose: () => void; children: ReactNode }) {
  const sheet = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  });
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    const box = sheet.current;
    const first = ["[data-current]", "a[aria-current]", "a, button"].map((selector) => box?.querySelector<HTMLElement>(selector)).find(Boolean);
    first?.focus({ preventScroll: true });
    function onKey(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.preventDefault();
      close.current();
    }
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      before?.focus?.({ preventScroll: true });
    };
  }, []);

  // In place rather than in a portal on the body: it is fixed to the window either way.
  return (
    <div className="fixed inset-0 z-50">
      <div aria-hidden className="absolute inset-0 bg-(--scrim)" onClick={onClose} />
      <div
        ref={sheet}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        onClick={(event) => {
          if ((event.target as HTMLElement).closest("a")) onClose();
        }}
        className="absolute inset-x-0 bottom-0 flex max-h-[80dvh] flex-col rounded-t-[16px] bg-sheet pb-[max(16px,env(safe-area-inset-bottom))] shadow-pop"
      >
        <div aria-hidden className="mx-auto mt-2 mb-1 h-1 w-9 shrink-0 rounded-full bg-line-strong" />
        <div className="min-h-0 overflow-y-auto px-4">{children}</div>
      </div>
    </div>
  );
}
