import { useEffect, useRef, useState, type ReactNode } from "react";
import { Pencil } from "lucide-react";
import { renameProject } from "@/api/home";
import { ApiError } from "@/api/client";
import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

// The App page's header: the project's name with a pencil that turns the title
// into an input. Save sends the new name and shows it at once; the sidebar and
// other titles follow on the project's live event.

/** The page header of a project's App page, with the title renamable inline. */
export function RenameTitle({
  workspaceId,
  projectId,
  name,
  description,
}: {
  workspaceId: string;
  projectId: string;
  name: string;
  description?: ReactNode;
}) {
  const [editing, setEditing] = useState(false);
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

  function open() {
    setText(current);
    setError(null);
    setEditing(true);
  }

  function close() {
    attempt.current += 1;
    setSaving(false);
    setEditing(false);
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
      setEditing(false);
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

  return (
    <PageHeader
      title={
        editing ? (
          <form
            className="space-y-2"
            onSubmit={(event) => {
              event.preventDefault();
              void save();
            }}
          >
            <div className="flex items-stretch gap-2">
              <Input
                autoFocus
                aria-label="Project name"
                aria-invalid={error ? true : undefined}
                value={text}
                className="h-auto w-[26rem] max-w-full font-display text-[2.125rem] leading-[1.1] md:text-[2.125rem]"
                onChange={(event) => {
                  setText(event.target.value);
                  setError(null);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Escape") close();
                }}
              />
              <Button type="submit" size="sm" className="h-auto" disabled={!canSave}>
                Save
              </Button>
              <Button type="button" variant="outline" size="sm" className="h-auto" onClick={close}>
                Cancel
              </Button>
            </div>
            {error && (
              <p role="alert" className="font-sans text-[13px] leading-normal tracking-normal text-destructive">
                {error}
              </p>
            )}
          </form>
        ) : (
          current
        )
      }
      description={description}
      actions={
        editing ? undefined : (
          <Button type="button" variant="outline" size="sm" onClick={open}>
            <Pencil aria-hidden className="size-3.5" />
            <span className="sr-only">Rename project</span>
          </Button>
        )
      }
    />
  );
}
