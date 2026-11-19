import { useEffect, useRef, useState } from "react";
import { Ellipsis, Pencil } from "lucide-react";
import { renameProject } from "@/api/home";
import { ApiError } from "@/api/client";
import { usePopover } from "@/components/shell/popover";
import { cn } from "@/lib/utils";
import { btn, btnIcon, btnPrimary, field } from "./rows";

// The project's title on its Overview, renamed in place: "Rename project" in
// the "…" menu turns the title into an input with Save and Cancel. Save sends
// the new name and shows it at once; the bar and other titles follow on the
// project's live event.

const item =
  "flex h-[30px] w-full items-center gap-2 rounded-control px-2 text-left whitespace-nowrap text-ink outline-none hover:bg-hover focus-visible:bg-hover [&>svg]:size-(--icon-sm) [&>svg]:shrink-0 [&>svg]:text-ink-2";

/** The "…" menu of the project's Overview. */
export function ProjectMenu({ onRename }: { onRename: () => void }) {
  const { open, toggle, setOpen, anchor, opener } = usePopover();
  return (
    <div ref={anchor} className="relative inline-flex">
      <button ref={opener} type="button" aria-haspopup="menu" aria-expanded={open} aria-label="More actions" title="More actions" onClick={toggle} className={btnIcon}>
        <Ellipsis aria-hidden />
      </button>
      {open && (
        <div role="menu" aria-label="Project" className="absolute top-[calc(100%+4px)] right-0 z-40 min-w-[200px] origin-top-right animate-pop-in rounded-card bg-sheet p-1 shadow-pop">
          <button
            type="button"
            role="menuitem"
            className={item}
            onClick={() => {
              setOpen(false);
              onRename();
            }}
          >
            <Pencil aria-hidden strokeWidth={2} />
            Rename project
          </button>
        </div>
      )}
    </div>
  );
}

/** The project's name as the page title (its h1), or while `editing` the input that renames it. */
export function ProjectTitle({
  workspaceId,
  projectId,
  name,
  editing,
  onEditing,
  className,
}: {
  workspaceId: string;
  projectId: string;
  name: string;
  editing: boolean;
  onEditing: (editing: boolean) => void;
  /** The title's classes, besides its size. */
  className?: string;
}) {
  const [text, setText] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The name that a save returned, good while the loaded name is still the one it replaced.
  const [saved, setSaved] = useState<{ from: string; to: string } | null>(null);
  // Bumped when an in-flight save stops mattering (Cancel, unmount); its answer is then ignored.
  const attempt = useRef(0);
  useEffect(
    () => () => {
      attempt.current += 1;
    },
    [],
  );
  const current = saved && saved.from === name ? saved.to : name;
  const trimmed = text.trim();
  const canSave = trimmed !== "" && trimmed !== current && !saving;

  // Opening starts from the name shown.
  const [opened, setOpened] = useState(editing);
  if (opened !== editing) {
    setOpened(editing);
    if (editing) {
      setText(current);
      setError(null);
    }
  }

  function close() {
    attempt.current += 1;
    setSaving(false);
    onEditing(false);
  }

  async function save() {
    if (!canSave) return;
    const mine = ++attempt.current;
    setSaving(true);
    setError(null);
    try {
      const project = await renameProject(workspaceId, projectId, trimmed);
      if (attempt.current !== mine) return;
      setSaved({ from: name, to: project.name });
      setSaving(false);
      onEditing(false);
    } catch (failure) {
      if (attempt.current !== mine) return;
      setSaving(false);
      setError(
        failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken"
          ? "Another project has this name"
          : "The project could not be renamed. Try again.",
      );
    }
  }

  if (!editing) return <h1 className={cn("min-w-0 text-title font-semibold", className)}>{current}</h1>;
  return (
    <form
      aria-label="Rename project"
      className="flex flex-wrap items-center gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <input
        autoFocus
        aria-label="Project name"
        aria-invalid={error ? true : undefined}
        value={text}
        className={cn(field, "w-[22rem] max-w-full text-title font-semibold tracking-[-0.012em] aria-invalid:border-danger")}
        onChange={(event) => {
          setText(event.target.value);
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") close();
        }}
      />
      <button type="submit" className={btnPrimary} disabled={!canSave}>
        Save
      </button>
      <button type="button" className={btn} onClick={close}>
        Cancel
      </button>
      {error && (
        <p role="alert" className="w-full text-body font-normal tracking-normal text-danger-ink">
          {error}
        </p>
      )}
    </form>
  );
}
