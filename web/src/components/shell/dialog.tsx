import { useEffect, useId, useRef, type FormEvent, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";

// The app's modal surfaces, after the prototype (tasks/ui-redesign/v2/components.css,
// .dialog and .drawer): a dialog centred high over a scrim, and a drawer that
// slides in at the right edge. Both have a head (title, an optional line under
// it, a close button), a body and a footer of actions on the frame colour.
// Esc and a click on the scrim close them, Tab stays inside, the first field
// (or the element marked data-autofocus) takes the focus, and the focus goes
// back where it was when they close. Given `onSubmit` the surface is a form:
// Enter in a field submits it.

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * The element modal surfaces and their menus are drawn in: a child of the body,
 * never the body itself. React listens on a portal's container, and a container
 * that holds the app's own root makes it walk the tree in a loop once the
 * portal is gone (seen in jsdom as a hang on the next focus).
 */
export function layer(): HTMLElement {
  let found = document.getElementById("omni-layer");
  if (!found) {
    found = document.createElement("div");
    found.id = "omni-layer";
    document.body.appendChild(found);
  }
  return found;
}

function focusables(box: HTMLElement): HTMLElement[] {
  return [...box.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((element) => !element.closest("[inert]"));
}

/** Focus in on open, back out on close, and the keys every modal surface answers: Esc closes, Tab cycles inside. */
function useModal(onClose: () => void) {
  const box = useRef<HTMLElement>(null);
  const close = useRef(onClose);
  close.current = onClose;

  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    const root = box.current;
    if (root) {
      const body = root.querySelector<HTMLElement>("[data-modal-body]") ?? root;
      const first =
        root.querySelector<HTMLElement>("[data-autofocus]") ??
        body.querySelector<HTMLElement>('input:not([disabled]):not([type="hidden"]), textarea:not([disabled]), [role="combobox"], [role="radio"][aria-checked="true"], [role="radio"]') ??
        focusables(body)[0] ??
        root;
      first.focus();
    }
    return () => {
      if (before && document.contains(before)) before.focus();
    };
  }, []);

  function onKeyDown(event: KeyboardEvent<HTMLElement>) {
    if (event.key === "Escape") {
      // An open menu inside closes first, on its own Esc.
      if (box.current?.querySelector('[role="menu"]')) return;
      event.preventDefault();
      event.stopPropagation();
      close.current();
      return;
    }
    if (event.key !== "Tab" || !box.current) return;
    const all = focusables(box.current);
    if (all.length === 0) return;
    const first = all[0];
    const last = all[all.length - 1];
    if (event.shiftKey && (document.activeElement === first || !box.current.contains(document.activeElement))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (document.activeElement === last || !box.current.contains(document.activeElement))) {
      event.preventDefault();
      first.focus();
    }
  }

  return { box, onKeyDown };
}

type SurfaceProps = {
  title: ReactNode;
  /** A line under the title: what this does. */
  description?: ReactNode;
  children?: ReactNode;
  /** The footer's buttons, Cancel first and the one primary last. */
  footer: ReactNode;
  /** Something at the start of the footer: a hint, an error. */
  footerStart?: ReactNode;
  onClose: () => void;
  /** Makes the surface a form: Enter in a field submits it. */
  onSubmit?: () => void;
  /** The accessible name when the title is not plain text. */
  label?: string;
  /** Buttons in the head before the close button: a "…" menu. */
  headActions?: ReactNode;
  /** A line above the title: a key, a status. */
  eyebrow?: ReactNode;
};

function Head({
  titleId,
  descriptionId,
  title,
  description,
  eyebrow,
  actions,
  onClose,
}: {
  titleId: string;
  descriptionId: string;
  title: ReactNode;
  description?: ReactNode;
  eyebrow?: ReactNode;
  actions?: ReactNode;
  onClose: () => void;
}) {
  return (
    <div className="flex items-start gap-3 px-5 pt-5">
      <div className="min-w-0 flex-1">
        {eyebrow && <div className="mb-1.5 flex min-h-6 flex-wrap items-center gap-x-2 gap-y-1 text-meta text-ink-3">{eyebrow}</div>}
        <h2 id={titleId} className="text-section leading-(--lh-title) font-semibold tracking-[-0.006em] text-ink">
          {title}
        </h2>
        {description && (
          <p id={descriptionId} className="mt-1.5 text-ink-2">
            {description}
          </p>
        )}
      </div>
      {actions && <div className="-mt-1 flex shrink-0 items-center gap-1">{actions}</div>}
      <button
        type="button"
        aria-label="Close"
        onClick={onClose}
        className="-mt-1 -mr-1.5 grid size-(--control-h) shrink-0 place-items-center rounded-control text-ink-2 transition-colors duration-(--t-fast) hover:bg-hover hover:text-ink focus-visible:shadow-focus focus-visible:outline-none"
      >
        <X aria-hidden className="size-(--icon)" />
      </button>
    </div>
  );
}

function Foot({ start, children }: { start?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex flex-none items-center justify-end gap-2 border-t border-line bg-frame px-5 py-3 [&>a]:h-(--control-h-lg) [&>a]:px-3 [&>button]:h-(--control-h-lg) [&>button]:px-3">
      {start && <div className="mr-auto min-w-0">{start}</div>}
      {children}
    </div>
  );
}

function Surface({
  kind,
  title,
  description,
  children,
  footer,
  footerStart,
  onClose,
  onSubmit,
  label,
  headActions,
  eyebrow,
  wide,
}: SurfaceProps & { kind: "dialog" | "drawer"; wide?: boolean }) {
  const titleId = useId();
  const descriptionId = useId();
  const { box, onKeyDown } = useModal(onClose);
  const submit = onSubmit
    ? (event: FormEvent) => {
        event.preventDefault();
        onSubmit();
      }
    : undefined;
  const As = onSubmit ? "form" : "div";
  const drawer = kind === "drawer";

  return createPortal(
    <div
      className={cn(
        "fixed inset-0 z-[60] grid animate-in bg-(--scrim) duration-(--t-pop) fade-in-0",
        drawer ? "place-items-stretch justify-items-end p-2" : "place-items-start justify-items-center px-4 pt-[14vh]",
      )}
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <As
        ref={box as never}
        role="dialog"
        aria-modal="true"
        aria-labelledby={label ? undefined : titleId}
        aria-label={label}
        aria-describedby={description ? descriptionId : undefined}
        tabIndex={-1}
        noValidate={onSubmit ? true : undefined}
        onSubmit={submit}
        onKeyDown={onKeyDown}
        className={cn(
          "flex min-h-0 flex-col overflow-hidden rounded-sheet bg-sheet text-ink shadow-pop outline-none",
          drawer
            ? "w-[min(460px,100%)] animate-in duration-200 ease-out-quint fade-in-0 slide-in-from-right-6"
            : cn("max-h-[80vh] animate-pop-in", wide ? "w-[min(560px,calc(100vw-32px))]" : "w-[min(480px,calc(100vw-32px))]"),
        )}
      >
        <div className={cn(drawer ? "border-b border-line pb-4" : !children && "pb-5")}>
          <Head titleId={titleId} descriptionId={descriptionId} title={title} description={description} eyebrow={eyebrow} actions={headActions} onClose={onClose} />
        </div>
        <div data-modal-body className={cn("min-h-0 overflow-y-auto p-5", drawer && "flex-1", !children && "hidden")}>
          {children}
        </div>
        <Foot start={footerStart}>{footer}</Foot>
      </As>
    </div>,
    layer(),
  );
}

/**
 * A modal dialog: a title, an optional line under it, the body and a footer
 * with Cancel and one primary. With `onSubmit` it is a form, so Enter submits.
 */
export function Dialog(props: SurfaceProps & { wide?: boolean }) {
  return <Surface kind="dialog" {...props} />;
}

/** A drawer at the right edge, inset like the sheet; its body scrolls. */
export function Drawer(props: SurfaceProps) {
  return <Surface kind="drawer" {...props} />;
}

/** A dialog's error line, at the start of its footer or under its fields. */
export function DialogError({ children }: { children: ReactNode }) {
  return (
    <p role="alert" className="text-meta text-danger-ink">
      {children}
    </p>
  );
}
