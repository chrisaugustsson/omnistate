import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Pencil, Trash2 } from "lucide-react";
import type { PartBody, TrackSummary } from "@/api/generated";
import { ApiError } from "@/api/client";
import { deletePart, renamePart } from "@/api/parts";
import { PartIcon } from "@/components/shell/part-icon";
import { PageHead } from "@/components/shell/page";
import { StateIcon, inFlight, stateLabel } from "@/components/shell/state-icon";
import { partTone } from "@/lib/identity";
import { useHome } from "@/lib/use-home";
import { cn } from "@/lib/utils";
import { CreatePartForm } from "./forms";
import { useProjectMap } from "./project-data";
import { btn, btnDanger, btnIcon, btnPrimary, EmptyRow, field, ScopePage } from "./rows";

// The project's Parts: a row per part (its icon, name and kind, what it is,
// how many features it has and what is in flight on it), with creating,
// renaming in place and deleting once confirmed.

type Params = { workspaceId: string; projectId: string };

const columns = "grid grid-cols-[20px_minmax(0,1fr)_96px_160px_64px] items-start gap-3 max-md:grid-cols-[20px_minmax(0,1fr)_64px]";

function messageOf(failure: unknown, what: string): string {
  if (failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken") return "Another part of the project has this name";
  if (failure instanceof ApiError && failure.status === 400) return "The name can not be blank";
  return `The part could not be ${what}. Try again.`;
}

/** What is in flight on a part: "1 planning, 1 building", each with its state icon; or that nothing is. */
function Flying({ tracks }: { tracks: readonly TrackSummary[] }) {
  const flying = tracks.filter(({ track }) => inFlight(track.state));
  if (flying.length === 0) return <span className="text-ink-3">Nothing in flight</span>;
  return (
    <span className="flex flex-col gap-0.5">
      {(["planning", "building"] as const).map((state) => {
        const count = flying.filter(({ track }) => track.state === state).length;
        if (count === 0) return null;
        return (
          <span key={state} className="inline-flex items-center gap-1.5 text-ink-2">
            <StateIcon state={state} />
            {count} {stateLabel(state).toLowerCase()}
          </span>
        );
      })}
    </span>
  );
}

function PartRow({
  part,
  parts,
  features,
  tracks,
  params,
  onChanged,
}: {
  part: PartBody;
  parts: readonly PartBody[];
  features: number;
  tracks: readonly TrackSummary[];
  params: Params;
  onChanged: () => void;
}) {
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

  const icon = <PartIcon name={part.name} kind={part.kind} tone={partTone(parts, part.id)} className="mt-0.5" />;

  if (mode === "rename") {
    const trimmed = text.trim();
    return (
      <li className={cn(columns, "border-t border-line py-2")}>
        {icon}
        <form
          className="col-span-4 flex flex-wrap items-center gap-2 max-md:col-span-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (trimmed && trimmed !== part.name && !busy) void run(() => renamePart(params.workspaceId, params.projectId, part.id, trimmed), "renamed");
          }}
        >
          <input
            autoFocus
            aria-label="Part name"
            value={text}
            className={cn(field, "max-w-xs")}
            onChange={(event) => {
              setText(event.target.value);
              setError(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") close();
            }}
          />
          <button type="submit" className={btnPrimary} disabled={!trimmed || trimmed === part.name || busy}>
            Save
          </button>
          <button type="button" className={btn} onClick={close}>
            Cancel
          </button>
          {error && (
            <p role="alert" className="w-full text-danger-ink">
              {error}
            </p>
          )}
        </form>
      </li>
    );
  }

  if (mode === "delete") {
    return (
      <li className={cn(columns, "border-t border-line bg-danger-tint/40 py-2")}>
        {icon}
        <div className="col-span-4 flex flex-wrap items-center gap-2 max-md:col-span-2">
          <span className="min-w-0 flex-1">
            Delete <span className="font-medium">{part.name}</span>? The part is hidden; its tracks stay.
          </span>
          <button
            type="button"
            className={btnDanger}
            disabled={busy}
            onClick={() => void run(() => deletePart(params.workspaceId, params.projectId, part.id), "deleted")}
          >
            Delete
          </button>
          <button type="button" className={btn} onClick={close}>
            Cancel
          </button>
          {error && (
            <p role="alert" className="w-full text-danger-ink">
              {error}
            </p>
          )}
        </div>
      </li>
    );
  }

  return (
    <li className={cn(columns, "group/part relative border-t border-line py-2.5 transition-colors duration-(--t-fast) hover:bg-frame hover:shadow-[-8px_0_0_var(--frame),8px_0_0_var(--frame)]")}>
      {icon}
      <div className="min-w-0">
        <Link
          to="/w/$workspaceId/projects/$projectId/parts/$partId"
          params={{ ...params, partId: part.id }}
          className="font-medium text-ink after:absolute after:inset-0 focus-visible:outline-none focus-visible:after:rounded-control focus-visible:after:shadow-focus"
        >
          {part.name}
        </Link>
        <span className="ml-1.5 text-meta text-ink-3">{part.kind}</span>
        {part.description && <p className="mt-0.5 truncate text-ink-3">{part.description.split(/\r?\n/, 1)[0]}</p>}
      </div>
      <span className="text-ink-2 tabular-nums max-md:hidden">
        {features} {features === 1 ? "feature" : "features"}
      </span>
      <span className="max-md:hidden">
        <Flying tracks={tracks} />
      </span>
      <span className="relative z-10 -my-1 flex justify-end gap-0.5 opacity-0 transition-opacity duration-(--t-fast) group-focus-within/part:opacity-100 group-hover/part:opacity-100 max-md:opacity-100">
        <button
          type="button"
          className={btnIcon}
          aria-label={`Rename ${part.name}`}
          title="Rename"
          onClick={() => {
            setText(part.name);
            setMode("rename");
          }}
        >
          <Pencil aria-hidden />
        </button>
        <button type="button" className={btnIcon} aria-label={`Delete ${part.name}`} title="Delete" onClick={() => setMode("delete")}>
          <Trash2 aria-hidden />
        </button>
      </span>
    </li>
  );
}

const WORDS = ["no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];

/** A small count in words ("two"), a larger one in figures. */
function counted(count: number): string {
  return WORDS[count] ?? String(count);
}

function lede(parts: readonly PartBody[]): string {
  if (parts.length === 0) return "The apps and services the project is made of. Each part has its own features, docs, tracks and decisions.";
  const apps = parts.filter((part) => part.kind === "app").length;
  const services = parts.length - apps;
  const said = [apps > 0 && `${counted(apps)} ${apps === 1 ? "app" : "apps"}`, services > 0 && `${counted(services)} ${services === 1 ? "service" : "services"}`]
    .filter(Boolean)
    .join(" and ");
  return `${said.charAt(0).toUpperCase()}${said.slice(1)} that change together. Each part has its own features, docs, tracks and decisions.`;
}

/** The project's Parts page. */
export function PartsPage({ workspaceId, projectId }: Params) {
  const params = { workspaceId, projectId };
  const { map, failed, reload } = useProjectMap(workspaceId, projectId);
  const { home } = useHome(workspaceId, projectId);
  const [creating, setCreating] = useState(false);
  const parts = map?.parts ?? null;

  return (
    <ScopePage>
      <PageHead
        title="Parts"
        actions={
          <button type="button" className={btnPrimary} aria-expanded={creating} onClick={() => setCreating(!creating)}>
            Create part
          </button>
        }
      />
      {parts && <p className="mt-1.5 max-w-(--main-max) text-em text-ink-2">{lede(parts)}</p>}
      {creating && (
        <CreatePartForm
          {...params}
          onDone={(created) => {
            setCreating(false);
            if (created) reload();
          }}
        />
      )}
      {failed && (
        <p role="alert" className="mt-4 text-danger-ink">
          The parts could not be loaded. Try again in a moment.
        </p>
      )}
      {parts && (
        <div className="mt-6">
          <div aria-hidden className={cn(columns, "h-8 items-center text-meta font-medium text-ink-3")}>
            <span />
            <span>Part</span>
            <span className="max-md:hidden">Features</span>
            <span className="max-md:hidden">In flight</span>
            <span />
          </div>
          <ul aria-label="Parts" className="border-b border-line">
            {parts.length === 0 ? (
              <EmptyRow>No parts yet. A part is an app or a service of the project; create one to start tracks on it.</EmptyRow>
            ) : (
              parts.map((part) => (
                <PartRow
                  key={part.id}
                  part={part}
                  parts={parts}
                  features={map?.features.get(part.id)?.length ?? 0}
                  tracks={(home?.tracks ?? []).filter(({ track }) => track.part_id === part.id)}
                  params={params}
                  onChanged={reload}
                />
              ))
            )}
          </ul>
        </div>
      )}
    </ScopePage>
  );
}
