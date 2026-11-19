import { useCallback, useEffect, useRef, useState } from "react";

/**
 * A popover's open state, closed by a pointer down outside `anchor` and by Esc
 * (which gives the focus back to `opener`). The anchor wraps the opener and the
 * popover; the popover renders while `open`.
 */
export function usePopover<Opener extends HTMLElement = HTMLButtonElement>() {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLDivElement>(null);
  const opener = useRef<Opener>(null);
  const close = useCallback((refocus = false) => {
    setOpen(false);
    if (refocus) opener.current?.focus();
  }, []);
  const toggle = useCallback(() => setOpen((now) => !now), []);

  useEffect(() => {
    if (!open) return;
    function outside(event: PointerEvent) {
      if (!anchor.current?.contains(event.target as Node)) setOpen(false);
    }
    function onKey(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setOpen(false);
      opener.current?.focus();
    }
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return { open, setOpen, toggle, close, anchor, opener };
}
