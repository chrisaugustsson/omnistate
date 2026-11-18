import { useState } from "react";
import { Pencil, Plus } from "lucide-react";
import { cn } from "@/lib/utils";
import { Textarea } from "@/components/shell/field";
import { btn, btnPrimary } from "./rows";

// The written intro of an Overview, as plain page text: the project's, the
// part's or the feature's description, its paragraphs split on blank lines. An editable intro
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
 * The intro. With `onSave` it can be edited; without, it is shown as it is and
 * an empty one shows nothing. A saved text shows until `text` changes, so the
 * old one does not flash back while the page reloads it.
 */
export function Intro({
  text,
  onSave,
  className,
}: {
  text: string | null | undefined;
  onSave?: (text: string) => Promise<void>;
  className?: string;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);
  const [saved, setSaved] = useState<{ was: string | null | undefined; text: string } | null>(null);
  const current = (saved && saved.was === text ? saved.text : text)?.trim() ?? "";

  function open() {
    setDraft(current);
    setFailed(false);
    setEditing(true);
  }

  async function save() {
    if (!onSave) return;
    if (draft.trim() === current) {
      setEditing(false);
      return;
    }
    setSaving(true);
    setFailed(false);
    try {
      await onSave(draft.trim());
      setSaved({ was: text, text: draft.trim() });
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
        className={cn("mt-4 max-w-[680px]", className)}
        onSubmit={(event) => {
          event.preventDefault();
          if (!saving) void save();
        }}
      >
        <Textarea
          autoFocus
          aria-label="Description"
          value={draft}
          minRows={2}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") setEditing(false);
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
              event.preventDefault();
              if (!saving) void save();
            }
          }}
          placeholder="What this is, who it is for, and where to start reading"
          className="-mx-2.5 w-[calc(100%+20px)] max-w-none border-line bg-transparent py-1.5 text-em leading-(--lh-prose) hover:border-line-strong"
        />
        {failed && (
          <p role="alert" className="mt-1 text-danger-ink">
            The description could not be saved. Try again.
          </p>
        )}
        <div className="mt-2 flex items-center gap-2">
          <button type="submit" className={btnPrimary} disabled={saving}>
            Save
          </button>
          <button type="button" className={btn} onClick={() => setEditing(false)}>
            Cancel
          </button>
          <span className="ml-1 text-meta text-ink-3">⌘ Enter saves, Esc cancels</span>
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
        className={cn("mt-3 -ml-1.5 inline-flex h-6 items-center gap-1.5 rounded-badge px-1.5 font-medium text-ink-3 transition-colors duration-(--t-fast) hover:bg-hover hover:text-ink focus-visible:shadow-focus focus-visible:outline-none", className)}
      >
        <Plus aria-hidden className="size-3" />
        Add a description
      </button>
    );
  }

  return (
    <div className={cn("group/intro relative mt-4 max-w-[680px] text-em leading-[1.6] text-ink", className)}>
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
