import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Box, Pencil, Trash2 } from "lucide-react";
import type { FeatureBody, TrackSummary } from "@/api/generated";
import { ApiError } from "@/api/client";
import { deleteFeature, renameFeature } from "@/api/features";
import { PageHead } from "@/components/shell/page";
import { StateIcon, inFlight } from "@/components/shell/state-icon";
import { useHome } from "@/lib/use-home";
import { useProjects } from "@/lib/use-workspace";
import { cn } from "@/lib/utils";
import { CreateFeatureForm, StartTrackForm } from "./forms";
import { PartNotFound } from "./overview";
import { isReplaced, useDecisionGroups, useProjectMap, type DecisionGroup } from "./project-data";
import { btn, btnDanger, btnIcon, btnPrimary, field, ScopePage, shortDate } from "./rows";

// A part's Features: what the part can do on main today, a card per feature
// with what it does, whether a track is changing it, and how many decisions
// and docs it has; with creating, renaming in place and deleting once
// confirmed. A feature's description is set when it is created, as in the CLI.

type Params = { workspaceId: string; projectId: string };

function messageOf(failure: unknown, what: string): string {
  if (failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken") return "Another feature of the part has this name";
  if (failure instanceof ApiError && failure.status === 400) return "The name can not be blank";
  return `The feature could not be ${what}. Try again.`;
}

const plural = (count: number, one: string) => `${count} ${count === 1 ? one : `${one}s`}`;

function FeatureCard({
  feature,
  partId,
  params,
  changing,
  group,
  docs,
  onChanged,
}: {
  feature: FeatureBody;
  partId: string;
  params: Params;
  /** The tracks in flight that change the feature. */
  changing: readonly TrackSummary[];
  group: DecisionGroup | undefined;
  docs: number;
  onChanged: () => void;
}) {
  const [mode, setMode] = useState<"view" | "rename" | "delete">("view");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const by = changing[0];
  const decisions = group?.decisions.filter((decision) => !isReplaced(decision)).length ?? 0;
  const replaced = group?.decisions.filter(isReplaced).length ?? 0;

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

  const trimmed = text.trim();
  return (
    <li className="group/feature flex min-w-0 flex-col overflow-hidden rounded-card border border-line bg-sheet">
      <div className="flex-1 p-4">
        {mode === "rename" ? (
          <form
            className="flex flex-wrap items-center gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              if (trimmed && trimmed !== feature.name && !busy)
                void run(() => renameFeature(params.workspaceId, params.projectId, partId, feature.id, trimmed), "renamed");
            }}
          >
            <input
              autoFocus
              aria-label="Feature name"
              value={text}
              className={cn(field, "min-w-40 flex-1")}
              onChange={(event) => {
                setText(event.target.value);
                setError(null);
              }}
              onKeyDown={(event) => {
                if (event.key === "Escape") close();
              }}
            />
            <button type="submit" className={btnPrimary} disabled={!trimmed || trimmed === feature.name || busy}>
              Save
            </button>
            <button type="button" className={btn} onClick={close}>
              Cancel
            </button>
          </form>
        ) : (
          <div className="flex items-center gap-2.5">
            <span className="grid size-7 shrink-0 place-items-center rounded-control border border-line bg-frame">
              <Box aria-hidden className="size-(--icon-sm) text-ink-2" strokeWidth={1.75} />
            </span>
            <Link
              to="/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId"
              params={{ ...params, partId, featureId: feature.id }}
              className="min-w-0 truncate text-section font-semibold text-ink hover:underline hover:decoration-line-strong hover:underline-offset-3"
            >
              {feature.name}
            </Link>
            <span className="ml-auto flex shrink-0 gap-0.5 opacity-0 transition-opacity duration-(--t-fast) group-focus-within/feature:opacity-100 group-hover/feature:opacity-100 max-md:opacity-100">
              <button
                type="button"
                className={btnIcon}
                aria-label={`Rename ${feature.name}`}
                title="Rename"
                onClick={() => {
                  setText(feature.name);
                  setMode("rename");
                }}
              >
                <Pencil aria-hidden />
              </button>
              <button type="button" className={btnIcon} aria-label={`Delete ${feature.name}`} title="Delete" onClick={() => setMode("delete")}>
                <Trash2 aria-hidden />
              </button>
            </span>
          </div>
        )}
        {mode === "delete" ? (
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <span className="w-full">
              Delete <span className="font-medium">{feature.name}</span>? The feature is hidden; its tracks and decisions stay.
            </span>
            <button
              type="button"
              className={btnDanger}
              disabled={busy}
              onClick={() => void run(() => deleteFeature(params.workspaceId, params.projectId, partId, feature.id), "deleted")}
            >
              Delete
            </button>
            <button type="button" className={btn} onClick={close}>
              Cancel
            </button>
          </div>
        ) : (
          <p className={cn("mt-3", feature.description ? "text-ink" : "text-ink-3")}>{feature.description || "No description."}</p>
        )}
        {error && (
          <p role="alert" className="mt-2 text-danger-ink">
            {error}
          </p>
        )}
      </div>
      <div className={cn("border-t border-line px-4 py-3 text-meta text-ink-3", by && "bg-brand-tint")}>
        {by ? (
          <p className="flex min-w-0 items-center gap-1.5 text-ink-2">
            <StateIcon state={by.track.state} className="size-3" />
            Being changed by
            <Link
              to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
              params={{ ...params, trackId: by.track.id }}
              className="min-w-0 truncate font-medium text-ink hover:underline"
            >
              {by.track.name}
            </Link>
            {changing.length > 1 && <span>and {changing.length - 1} more</span>}
          </p>
        ) : (
          <p>Nothing in flight · added {shortDate(feature.created_at)}</p>
        )}
        <p className="mt-1.5 flex gap-3 tabular-nums">
          <span>
            {plural(decisions, "decision")}
            {replaced > 0 && `, ${replaced} replaced`}
          </span>
          <span>{plural(docs, "doc")}</span>
        </p>
      </div>
    </li>
  );
}

/** A part's Features page. */
export function FeaturesPage({ workspaceId, projectId, partId }: Params & { partId: string }) {
  const params = { workspaceId, projectId };
  const { projects } = useProjects(workspaceId, projectId);
  const projectName = projects?.find((candidate) => candidate.id === projectId)?.name;
  const { map, failed, reload } = useProjectMap(workspaceId, projectId);
  const { home, reload: reloadHome } = useHome(workspaceId, projectId);
  const { groups } = useDecisionGroups(workspaceId, projectId, projectName, map, partId);
  const [opened, setOpened] = useState<"feature" | "track" | null>(null);

  const part = map?.parts.find((candidate) => candidate.id === partId);
  if (map && !part) return <PartNotFound params={params} />;
  const features = map?.features.get(partId) ?? null;
  const tracks = home?.tracks ?? [];
  const changing = (featureId: string) =>
    (map?.changedBy.get(featureId) ?? [])
      .map((trackId) => tracks.find((summary) => summary.track.id === trackId))
      .filter((summary): summary is TrackSummary => Boolean(summary && inFlight(summary.track.state)));

  return (
    <ScopePage>
      <PageHead
        title="Features"
        actions={
          part && (
            <>
              <button type="button" className={btn} aria-expanded={opened === "track"} onClick={() => setOpened(opened === "track" ? null : "track")}>
                Start track
              </button>
              <button type="button" className={btnPrimary} aria-expanded={opened === "feature"} onClick={() => setOpened(opened === "feature" ? null : "feature")}>
                Create feature
              </button>
            </>
          )
        }
      />
      {part && <p className="mt-1.5 max-w-(--main-max) text-em text-ink-2">What {part.name} can do on main today. To change one, start a track on it.</p>}
      {opened === "feature" && (
        <CreateFeatureForm
          {...params}
          partId={partId}
          partName={part?.name}
          onDone={(created) => {
            setOpened(null);
            if (created) reload();
          }}
        />
      )}
      {opened === "track" && map && (
        <StartTrackForm
          {...params}
          fixedPartId={partId}
          parts={map.parts}
          onDone={(created) => {
            setOpened(null);
            if (created) reloadHome();
          }}
        />
      )}
      {failed && (
        <p role="alert" className="mt-4 text-danger-ink">
          The features could not be loaded. Try again in a moment.
        </p>
      )}
      {features &&
        (features.length === 0 ? (
          <p className="mt-6 border-y border-line py-2.5 text-ink-3">No features yet. A feature is something {part?.name} can do; create one to say what.</p>
        ) : (
          <ul aria-label="Features" className="mt-8 grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
            {features.map((feature) => (
              <FeatureCard
                key={feature.id}
                feature={feature}
                partId={partId}
                params={params}
                changing={changing(feature.id)}
                group={groups?.find((group) => group.subject.id === feature.id)}
                docs={(home?.docs ?? []).filter((doc) => doc.feature_id === feature.id).length}
                onChanged={reload}
              />
            ))}
          </ul>
        ))}
    </ScopePage>
  );
}
