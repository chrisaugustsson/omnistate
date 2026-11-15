import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Pencil, Plus, Trash2 } from "lucide-react";
import type { PartBody } from "@/api/generated";
import { ApiError } from "@/api/client";
import { createPart, deletePart, renamePart } from "@/api/parts";
import { Empty, List, rowClass } from "@/components/project/items";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useParts } from "@/lib/use-parts";

// The parts of a project (D2): the apps and services, each with its name and
// kind, and a way to create, rename and delete one. The page reloads the list
// on the project's live events, so a part made elsewhere shows up here.

type Params = { workspaceId: string; projectId: string };

const selectClass =
  "h-9 rounded-md border bg-background px-3 text-sm shadow-xs outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50";

/** The kind of a part as a small label. */
export function KindLabel({ kind }: { kind: string }) {
  return (
    <span className="inline-flex shrink-0 items-center rounded-md px-1.5 py-px text-[11px] font-medium text-muted-foreground ring-1 ring-border ring-inset">
      {kind}
    </span>
  );
}

function messageOf(failure: unknown, what: string): string {
  if (failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken")
    return "Another part of the project has this name";
  if (failure instanceof ApiError && failure.status === 400) return "The name can not be blank";
  return `The part could not be ${what}. Try again.`;
}

function CreateForm({ workspaceId, projectId, onDone }: Params & { onDone: () => void }) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState("");
  const [description, setDescription] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const canSave = name.trim() !== "" && kind !== "" && !saving;

  async function save() {
    if (!canSave) return;
    setSaving(true);
    setError(null);
    try {
      await createPart(workspaceId, projectId, { name: name.trim(), kind, description: description.trim() });
      onDone();
    } catch (failure) {
      setSaving(false);
      setError(messageOf(failure, "created"));
    }
  }

  return (
    <form
      className="space-y-3 rounded-xl border bg-card p-4"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <div className="flex flex-wrap gap-2">
        <Input
          autoFocus
          aria-label="Part name"
          placeholder="Name"
          value={name}
          className="max-w-xs flex-1"
          onChange={(event) => {
            setName(event.target.value);
            setError(null);
          }}
        />
        <select aria-label="Part kind" value={kind} onChange={(event) => setKind(event.target.value)} className={selectClass}>
          <option value="" disabled>
            Kind
          </option>
          <option value="app">app</option>
          <option value="service">service</option>
        </select>
      </div>
      <Input
        aria-label="Part description"
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
          Create part
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

function PartRow({ part, params, onChanged }: { part: PartBody; params: Params; onChanged: () => void }) {
  const [mode, setMode] = useState<"view" | "rename" | "delete">("view");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function close() {
    setMode("view");
    setError(null);
    setBusy(false);
  }

  async function run(action: () => Promise<unknown>, what: string) {
    setBusy(true);
    setError(null);
    try {
      await action();
      close();
      onChanged();
    } catch (failure) {
      setBusy(false);
      setError(messageOf(failure, what));
    }
  }

  if (mode === "rename") {
    const trimmed = text.trim();
    return (
      <li className="space-y-2 px-4 py-3">
        <form
          className="flex gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (trimmed && trimmed !== part.name && !busy)
              void run(() => renamePart(params.workspaceId, params.projectId, part.id, trimmed), "renamed");
          }}
        >
          <Input
            autoFocus
            aria-label="Part name"
            value={text}
            className="max-w-xs"
            onChange={(event) => {
              setText(event.target.value);
              setError(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") close();
            }}
          />
          <Button type="submit" size="sm" disabled={!trimmed || trimmed === part.name || busy}>
            Save
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={close}>
            Cancel
          </Button>
        </form>
        {error && (
          <p role="alert" className="text-[13px] text-destructive">
            {error}
          </p>
        )}
      </li>
    );
  }

  if (mode === "delete") {
    return (
      <li className="space-y-2 px-4 py-3 text-sm">
        <div className="flex flex-wrap items-center gap-2">
          <span className="min-w-0 flex-1">
            Delete <span className="font-medium">{part.name}</span>? The part is hidden; its tracks stay.
          </span>
          <Button
            type="button"
            variant="destructive"
            size="sm"
            disabled={busy}
            onClick={() => void run(() => deletePart(params.workspaceId, params.projectId, part.id), "deleted")}
          >
            Delete
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={close}>
            Cancel
          </Button>
        </div>
        {error && (
          <p role="alert" className="text-[13px] text-destructive">
            {error}
          </p>
        )}
      </li>
    );
  }

  return (
    <li className="flex items-center transition-colors hover:bg-accent/60">
      <Link
        to="/w/$workspaceId/projects/$projectId/parts/$partId"
        params={{ ...params, partId: part.id }}
        className={`${rowClass} min-w-0 flex-1 hover:bg-transparent`}
      >
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium">{part.name}</span>
          {part.description && (
            <span className="mt-0.5 block truncate text-[13px] text-muted-foreground">{part.description.split(/\r?\n/, 1)[0]}</span>
          )}
        </span>
        <KindLabel kind={part.kind} />
      </Link>
      <div className="flex shrink-0 gap-1 pr-3">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => {
            setText(part.name);
            setMode("rename");
          }}
        >
          <Pencil aria-hidden className="size-3.5" />
          <span className="sr-only">Rename {part.name}</span>
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={() => setMode("delete")}>
          <Trash2 aria-hidden className="size-3.5" />
          <span className="sr-only">Delete {part.name}</span>
        </Button>
      </div>
    </li>
  );
}

/** The project's parts with a form to create one. */
export function PartsSection({ workspaceId, projectId }: Params) {
  const params = { workspaceId, projectId };
  const { parts, failed, reload } = useParts(workspaceId, projectId);
  const [creating, setCreating] = useState(false);

  return (
    <section id="parts" aria-labelledby="parts-title" className="scroll-mt-6 space-y-3">
      <div className="flex items-center gap-2.5">
        <h2 id="parts-title" className="text-[15px] font-semibold tracking-tight">
          Parts
        </h2>
        {parts && <span className="text-sm text-muted-foreground tabular-nums">{parts.length}</span>}
        {!creating && (
          <Button type="button" variant="outline" size="sm" className="ml-auto" onClick={() => setCreating(true)}>
            <Plus aria-hidden className="size-3.5" />
            New part
          </Button>
        )}
      </div>
      {failed && (
        <Alert variant="destructive">
          <AlertDescription>The parts could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {creating && (
        <CreateForm
          {...params}
          onDone={() => {
            setCreating(false);
            reload();
          }}
        />
      )}
      {parts &&
        (parts.length === 0 ? (
          <Empty>No parts yet</Empty>
        ) : (
          <List>
            {parts.map((part) => (
              <PartRow key={part.id} part={part} params={params} onChanged={reload} />
            ))}
          </List>
        ))}
    </section>
  );
}
