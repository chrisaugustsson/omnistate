import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import type { PartBody } from "@/api/generated";
import { ApiError } from "@/api/client";
import { createTrack } from "@/api/parts";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

// Creating a track (D2): on a part page the track goes on that part; on the
// project-wide list the form asks for the part, which starts empty and is
// required.

const STATE_TONES: Record<string, string> = {
  planning: "text-muted-foreground ring-border",
  building: "text-sky-700 ring-sky-300 dark:text-sky-300 dark:ring-sky-800",
  done: "text-emerald-700 ring-emerald-300 dark:text-emerald-400 dark:ring-emerald-800",
  dropped: "text-muted-foreground ring-border line-through",
};

/** A track's state as a small label. */
export function StateLabel({ state }: { state: string }) {
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center rounded-md px-1.5 py-px text-[11px] font-medium ring-1 ring-inset",
        STATE_TONES[state] ?? STATE_TONES.planning,
      )}
    >
      {state}
    </span>
  );
}

const selectClass =
  "h-9 rounded-md border bg-background px-3 text-sm shadow-xs outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50";

type Params = { workspaceId: string; projectId: string };

/**
 * The create action for a track. With `fixedPartId` the track goes on that
 * part; with `parts` it asks for one. With no parts to pick from the action is
 * disabled and points to the project page.
 */
export function NewTrack({
  workspaceId,
  projectId,
  fixedPartId,
  parts,
  onCreated,
}: Params & { fixedPartId?: string; parts?: PartBody[]; onCreated: () => void }) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [partId, setPartId] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const part = fixedPartId ?? partId;
  const canSave = name.trim() !== "" && part !== "" && !saving;

  if (!fixedPartId && parts && parts.length === 0) {
    return (
      <div className="flex items-center gap-3">
        <Button type="button" size="sm" disabled>
          <Plus aria-hidden className="size-3.5" />
          New track
        </Button>
        <span className="text-sm text-muted-foreground">
          A track goes on a part.{" "}
          <Link
            to="/w/$workspaceId/projects/$projectId/app"
            params={{ workspaceId, projectId }}
            hash="parts"
            className="font-medium text-foreground underline underline-offset-4"
          >
            Create a part on the project page
          </Link>
        </span>
      </div>
    );
  }

  if (!open) {
    return (
      <Button type="button" size="sm" onClick={() => setOpen(true)}>
        <Plus aria-hidden className="size-3.5" />
        New track
      </Button>
    );
  }

  async function save() {
    if (!canSave) return;
    setSaving(true);
    setError(null);
    try {
      await createTrack(workspaceId, projectId, { name: name.trim(), description: description.trim(), part_id: part });
      setOpen(false);
      setSaving(false);
      setName("");
      setDescription("");
      setPartId("");
      onCreated();
    } catch (failure) {
      setSaving(false);
      setError(
        failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken"
          ? "Another track of the project has this name"
          : "The track could not be created. Try again.",
      );
    }
  }

  return (
    <form
      className="w-full space-y-3 rounded-xl border bg-card p-4"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <div className="flex flex-wrap gap-2">
        <Input
          autoFocus
          aria-label="Track name"
          placeholder="Name"
          value={name}
          className="max-w-xs flex-1"
          onChange={(event) => {
            setName(event.target.value);
            setError(null);
          }}
        />
        {!fixedPartId && (
          <select aria-label="Part" value={partId} onChange={(event) => setPartId(event.target.value)} className={selectClass}>
            <option value="" disabled>
              Part
            </option>
            {(parts ?? []).map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.name}
              </option>
            ))}
          </select>
        )}
      </div>
      <Input
        aria-label="Track description"
        placeholder="Description (optional)"
        value={description}
        onChange={(event) => setDescription(event.target.value)}
      />
      {error && (
        <p role="alert" className="text-[13px] text-destructive">
          {error}
        </p>
      )}
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={!canSave}>
          Create track
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={() => setOpen(false)}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
