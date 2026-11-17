import { useState } from "react";
import { ChevronDown, ListFilter } from "lucide-react";
import type { TrackSummary } from "@/api/generated";
import { PageHead } from "@/components/shell/page";
import { StateIcon, TRACK_STATES, stateLabel } from "@/components/shell/state-icon";
import { useHome } from "@/lib/use-home";
import { useProjects } from "@/lib/use-workspace";
import { cn } from "@/lib/utils";
import { StartTrackForm } from "./forms";
import { PartNotFound } from "./overview";
import { useMemberNames, useRowFlags, useProjectMap } from "./project-data";
import { Band, btnPrimary, ScopePage } from "./rows";
import { TrackLine } from "./track-lines";

// The Tracks of a project or a part, grouped by state like Linear: a band per
// state (Planning, Building, Done, Dropped) with its count, the most recently
// active track first in each. A filter bar narrows them by state, owner and
// (on the project) part. Start track opens the form under the head.

type Params = { workspaceId: string; projectId: string };

const activity = ({ track, last_activity }: TrackSummary) => Date.parse(last_activity ?? track.created_at) || 0;
const byActivity = (a: TrackSummary, b: TrackSummary) => activity(b) - activity(a) || a.track.name.localeCompare(b.track.name);

/** A filter of the bar: a compact select that reads as a quiet button. */
function FilterSelect({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: readonly { value: string; label: string }[];
}) {
  return (
    <label
      className={cn(
        "relative inline-flex h-(--control-h) items-center gap-1 rounded-control pr-1.5 pl-2 font-medium transition-colors duration-(--t-fast) hover:bg-hover focus-within:shadow-focus",
        value ? "bg-selected text-ink" : "text-ink-2",
      )}
    >
      <span className="sr-only">{label}</span>
      <select value={value} onChange={(event) => onChange(event.target.value)} className="absolute inset-0 cursor-pointer appearance-none opacity-0">
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      <span aria-hidden>{options.find((option) => option.value === value)?.label}</span>
      <ChevronDown aria-hidden className="size-3 text-ink-3" />
    </label>
  );
}

/** The Tracks page of a project, or with `partId` of a part. */
export function TracksPage({ workspaceId, projectId, partId }: Params & { partId?: string }) {
  const params = { workspaceId, projectId };
  const { projects } = useProjects(workspaceId, projectId);
  const project = projects?.find((candidate) => candidate.id === projectId);
  const { home, failed, reload } = useHome(workspaceId, projectId);
  const { map } = useProjectMap(workspaceId, projectId);
  const names = useMemberNames(workspaceId);
  const [starting, setStarting] = useState(false);
  const [state, setState] = useState("");
  const [owner, setOwner] = useState("");
  const [part, setPart] = useState("");

  const all = home?.tracks ?? [];
  const scoped = partId ? all.filter(({ track }) => track.part_id === partId) : all;
  const questions = useRowFlags(workspaceId, home ? scoped : null);
  const thePart = partId ? map?.parts.find((candidate) => candidate.id === partId) : undefined;
  if (partId && map && !thePart) return <PartNotFound params={params} />;

  const shown = scoped.filter(
    ({ track }) => (!state || track.state === state) && (!owner || track.owner_user_id === owner) && (!part || track.part_id === part),
  );
  const owners = [...new Set(scoped.map(({ track }) => track.owner_user_id).filter((id): id is string => Boolean(id)))];
  const filtered = Boolean(state || owner || part);
  const lede = partId
    ? `Work on ${thePart?.name ?? "the part"}, active first. Done and dropped tracks stay listed with what they changed.`
    : `Every piece of work in ${project?.name ?? "the project"}, one track per part. In flight first; done and dropped stay listed.`;

  return (
    <ScopePage>
      <PageHead
        title="Tracks"
        actions={
          <button type="button" className={btnPrimary} aria-expanded={starting} onClick={() => setStarting(!starting)}>
            Start track
          </button>
        }
      />
      <p className="mt-1.5 max-w-(--main-max) text-em text-ink-2">{lede}</p>
      {starting && map && (
        <StartTrackForm
          {...params}
          fixedPartId={partId}
          parts={map.parts}
          onDone={(created) => {
            setStarting(false);
            if (created) reload();
          }}
        />
      )}
      {failed && (
        <p role="alert" className="mt-4 text-danger-ink">
          The tracks could not be loaded. Try again in a moment.
        </p>
      )}
      {home && (
        <>
          <div role="group" aria-label="Filter" className="mt-6 flex flex-wrap items-center gap-1">
            <ListFilter aria-hidden className="mr-1 size-(--icon-sm) text-ink-3" />
            <FilterSelect
              label="State"
              value={state}
              onChange={setState}
              options={[{ value: "", label: "Any state" }, ...TRACK_STATES.map((one) => ({ value: one, label: stateLabel(one) }))]}
            />
            <FilterSelect
              label="Owner"
              value={owner}
              onChange={setOwner}
              options={[{ value: "", label: "Anyone" }, ...owners.map((id) => ({ value: id, label: names?.get(id) ?? "Owner" }))]}
            />
            {!partId && (
              <FilterSelect
                label="Part"
                value={part}
                onChange={setPart}
                options={[{ value: "", label: "Any part" }, ...(map?.parts ?? []).map((one) => ({ value: one.id, label: one.name }))]}
              />
            )}
            {filtered && (
              <button
                type="button"
                className="ml-1 h-6 rounded-badge px-1.5 text-meta font-medium text-ink-3 hover:bg-hover hover:text-ink"
                onClick={() => {
                  setState("");
                  setOwner("");
                  setPart("");
                }}
              >
                Clear
              </button>
            )}
            <span aria-live="polite" className="ml-auto text-meta text-ink-3 tabular-nums">
              {filtered ? `${shown.length} of ${scoped.length}` : scoped.length} {scoped.length === 1 ? "track" : "tracks"}
            </span>
          </div>
          {scoped.length === 0 ? (
            <p className="mt-3 border-y border-line py-2.5 text-ink-3">
              No tracks yet. A track is one piece of work on a part; start one to change something.
            </p>
          ) : shown.length === 0 ? (
            <p className="mt-3 border-y border-line py-2.5 text-ink-3">No track matches the filter.</p>
          ) : (
            <div className="-mx-3 mt-3">
              {TRACK_STATES.map((band) => {
                const tracks = shown.filter(({ track }) => track.state === band).sort(byActivity);
                if (tracks.length === 0) return null;
                return (
                  <section key={band} aria-label={stateLabel(band)} className="mb-2">
                    <Band icon={<StateIcon state={band} />} count={tracks.length}>
                      {stateLabel(band)}
                    </Band>
                    <ul className="border-b border-line [&>li:first-child>a]:border-t-0">
                      {tracks.map((summary) => (
                        <TrackLine
                          key={summary.track.id}
                          summary={summary}
                          all={all}
                          params={params}
                          map={map}
                          names={names}
                          questions={questions}
                          showPart={!partId}
                          openThreads={home?.threads.filter((thread) => thread.track_id === summary.track.id && !thread.conclusion).length}
                        />
                      ))}
                    </ul>
                  </section>
                );
              })}
            </div>
          )}
        </>
      )}
    </ScopePage>
  );
}
