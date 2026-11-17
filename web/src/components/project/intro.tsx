import { useState } from "react";
import { Pencil, Plus } from "lucide-react";
import { cn } from "@/lib/utils";
import { btn, btnPrimary, field } from "./rows";

// The written intro of an Overview, as plain page text: the project's or the
// part's description, its paragraphs split on blank lines. An editable intro
// has an Edit link at its top right that shows on hover; an empty one is a
// quiet "Add a description" link. Editing happens in place.

function Paragraphs({ text }: { text: string }) {
  const paragraphs = text
    .split(/\n\s*\n/)
    .map((paragraph) => paragraph.trim())
    .filter(Boolean);
  return (
    <>
      {paragraphs.map((paragraph, at) => (
        <p key={at} className="whitespace-pre-line [&+&]:mt-2">
          {paragraph}
        </p>
      ))}
    </>
  );
}

/**
 * The intro. With `onSave` it can be edited (the project's); without, it is
 * shown as it is and an empty one shows nothing (a part's, which the API does
 * not let the page change).
 */
export function Intro({ text, onSave }: { text: string | null | undefined; onSave?: (text: string) => Promise<void> }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);
  const current = text?.trim() ?? "";

  function open() {
    setDraft(current);
    setFailed(false);
    setEditing(true);
  }

  async function save() {
    if (!onSave) return;
    setSaving(true);
    setFailed(false);
    try {
      await onSave(draft.trim());
      setEditing(false);
    } catch {
      setFailed(true);
    } finally {
      setSaving(false);
    }
  }

  if (editing) {
    return (
      <form
        aria-label="Edit the description"
        className="mt-4 max-w-[680px] space-y-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (!saving) void save();
        }}
      >
        <textarea
          autoFocus
          aria-label="Description"
          value={draft}
          rows={Math.min(12, Math.max(4, draft.split("\n").length + 1))}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") setEditing(false);
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
              event.preventDefault();
              if (!saving) void save();
            }
          }}
          placeholder="What this is, who it is for, and where to start reading"
          className={cn(field, "h-auto py-2 text-em leading-[1.6]")}
        />
        {failed && (
          <p role="alert" className="text-danger-ink">
            The description could not be saved. Try again.
          </p>
        )}
        <div className="flex gap-2">
          <button type="submit" className={btnPrimary} disabled={saving || draft.trim() === current}>
            Save
          </button>
          <button type="button" className={btn} onClick={() => setEditing(false)}>
            Cancel
          </button>
        </div>
      </form>
    );
  }

  if (!current) {
    if (!onSave) return null;
    return (
      <button
        type="button"
        onClick={open}
        className="mt-3 -ml-1.5 inline-flex h-6 items-center gap-1.5 rounded-badge px-1.5 font-medium text-ink-3 transition-colors duration-(--t-fast) hover:bg-hover hover:text-ink focus-visible:shadow-focus focus-visible:outline-none"
      >
        <Plus aria-hidden className="size-3" />
        Add a description
      </button>
    );
  }

  return (
    <div className="group/intro relative mt-4 max-w-[680px] text-em leading-[1.6] text-ink">
      {onSave && (
        <button
          type="button"
          onClick={open}
          aria-label="Edit the description"
          className="absolute top-0 -right-12 inline-flex h-6 items-center gap-1 rounded-badge px-1.5 text-meta font-medium text-ink-3 opacity-0 transition-[opacity,background-color] duration-(--t-fast) group-hover/intro:opacity-100 hover:bg-hover hover:text-ink focus-visible:opacity-100 focus-visible:shadow-focus focus-visible:outline-none"
        >
          <Pencil aria-hidden className="size-3" />
          Edit
        </button>
      )}
      <Paragraphs text={current} />
    </div>
  );
}
