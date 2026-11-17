import { useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { Box, Layers, Plus } from "lucide-react";
import type { HomeResponse, PartBody, TrackSummary } from "@/api/generated";
import { describeProject } from "@/api/home";
import { describePart } from "@/api/parts";
import type { TrackFlags } from "@/components/home/use-home-data";
import { PartIcon, ProjectMark } from "@/components/shell/part-icon";
import { PageColumns, PageHead } from "@/components/shell/page";
import { PanelSection, PropertiesPanel, PropertyRow } from "@/components/shell/properties-panel";
import { StateIcon, inFlight, stateLabel } from "@/components/shell/state-icon";
import { partLink, projectLink } from "@/components/shell/scope";
import { partTone } from "@/lib/identity";
import { useHome } from "@/lib/use-home";
import { useProjects } from "@/lib/use-workspace";
import { RecentDecisionRow, type DecisionContext } from "./decision-log";
import { DocTiles, byRecent, partDocs, projectDocs } from "./docs-pane";
import { CreateFeatureForm, CreatePartForm, StartTrackForm } from "./forms";
import { Intro } from "./intro";
import { ProjectMenu, ProjectTitle } from "./project-title";
import { currentDecisions, useDecisionGroups, useMemberNames, useRowFlags, useProjectMap, type ProjectMap } from "./project-data";
import { btn, btnGhostSm, btnPrimary, ColumnList, CountLink, Counts, EmptyRow, rowClass, RowLead, ScopePage, Section, Sep, shortDate } from "./rows";
import { FlightRow } from "./track-lines";

// The Overview of a project and of a part: the title with its mark, the intro
// as plain page text, the tracks in flight, (a part's) features, the latest
// decisions and docs, and on the right the properties panel with the counts.

type Params = { workspaceId: string; projectId: string };

const activity = ({ track, last_activity }: TrackSummary) => Date.parse(last_activity ?? track.created_at) || 0;
const byActivity = (a: TrackSummary, b: TrackSummary) => activity(b) - activity(a) || a.track.name.localeCompare(b.track.name);

/** The page could not be loaded: one quiet line. */
function Failed() {
  return (
    <p role="alert" className="mt-4 rounded-control bg-danger-tint px-3 py-2 text-danger-ink">
      Part of the page could not be loaded. Try again in a moment.
    </p>
  );
}

/** "Last finished: X, done 4 Nov · Y, dropped": the two tracks that ended most recently. */
function LastFinished({ tracks, params }: { tracks: readonly TrackSummary[]; params: Params }) {
  const ended = tracks.filter(({ track }) => !inFlight(track.state)).sort(byActivity).slice(0, 2);
  if (ended.length === 0) return null;
  return (
    <p className="mt-2 text-meta text-ink-3">
      Last finished:{" "}
      {ended.map((summary, at) => (
        <span key={summary.track.id}>
          {at > 0 && <Sep />}
          <Link
            to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
            params={{ ...params, trackId: summary.track.id }}
            className="font-medium text-ink-2 hover:text-ink"
          >
            {summary.track.name}
          </Link>
          , {stateLabel(summary.track.state).toLowerCase()}
          {summary.last_activity && ` ${shortDate(summary.last_activity)}`}
        </span>
      ))}
    </p>
  );
}

/** The In flight section: the tracks being planned or built, the most recently active first. */
function InFlight({
  tracks,
  all,
  params,
  map,
  names,
  questions,
  showPart,
  allLink,
  empty,
}: {
  tracks: readonly TrackSummary[];
  all: readonly TrackSummary[];
  params: Params;
  map: ProjectMap | null;
  names: ReadonlyMap<string, string> | null;
  questions: ReadonlyMap<string, TrackFlags> | null;
  showPart: boolean;
  allLink: ReactNode;
  empty: ReactNode;
}) {
  const flying = tracks.filter(({ track }) => inFlight(track.state)).sort(byActivity);
  return (
    <Section id="in-flight" title="In flight" count={flying.length} end={allLink} className="mt-10">
      <ColumnList label="Tracks in flight">
        {flying.length === 0 ? (
          <EmptyRow icon={<Layers aria-hidden className="size-(--icon) text-faint" strokeWidth={1.5} />}>{empty}</EmptyRow>
        ) : (
          flying.map((summary) => (
            <FlightRow
              key={summary.track.id}
              summary={summary}
              all={all}
              params={params}
              map={map}
              names={names}
              questions={questions}
              showPart={showPart}
            />
          ))
        )}
      </ColumnList>
      <LastFinished tracks={tracks} params={params} />
    </Section>
  );
}

/** A part as a link in the panel: its icon, name and kind. */
function PartResource({ part, parts, params }: { part: PartBody; parts: readonly PartBody[]; params: Params }) {
  return (
    <Link
      {...partLink(params.workspaceId, params.projectId, part.id, "overview")}
      className="-ml-1 flex h-7 w-full items-center gap-2 rounded-control px-1 font-medium text-ink transition-colors duration-(--t-fast) hover:bg-hover"
    >
      <PartIcon name={part.name} kind={part.kind} tone={partTone(parts, part.id)} size="sm" />
      <span className="min-w-0 truncate">{part.name}</span>
      <span className="ml-auto text-meta font-normal text-ink-3">{part.kind}</span>
    </Link>
  );
}

function kinds(parts: readonly PartBody[]): string {
  const apps = parts.filter((part) => part.kind === "app").length;
  const services = parts.length - apps;
  const said = [apps > 0 && `${apps} ${apps === 1 ? "app" : "apps"}`, services > 0 && `${services} ${services === 1 ? "service" : "services"}`].filter(Boolean);
  return said.length > 0 ? said.join(", ") : "No parts yet";
}

type Opened = "part" | "feature" | "track" | null;

/** The head's buttons: a secondary create and Start track; each opens its form under the head. */
function HeadActions({ secondary, opened, onOpen }: { secondary: "part" | "feature"; opened: Opened; onOpen: (form: Opened) => void }) {
  return (
    <>
      <button type="button" className={btn} aria-expanded={opened === secondary} onClick={() => onOpen(opened === secondary ? null : secondary)}>
        <Plus aria-hidden />
        {secondary === "part" ? "Create part" : "Create feature"}
      </button>
      <button type="button" className={btnPrimary} aria-expanded={opened === "track"} onClick={() => onOpen(opened === "track" ? null : "track")}>
        Start track
      </button>
    </>
  );
}

/** A project's Overview. */
export function ProjectOverview({ workspaceId, projectId }: Params) {
  const params = { workspaceId, projectId };
  const { projects, failed: projectsFailed, reload: reloadProjects } = useProjects(workspaceId, projectId);
  const project = projects?.find((candidate) => candidate.id === projectId);
  const { home, failed: homeFailed, reload: reloadHome } = useHome(workspaceId, projectId);
  const { map, failed: mapFailed, reload: reloadMap } = useProjectMap(workspaceId, projectId);
  const { groups } = useDecisionGroups(workspaceId, projectId, project?.name, map);
  const names = useMemberNames(workspaceId);
  const tracks = home?.tracks ?? [];
  const questions = useRowFlags(workspaceId, home?.tracks ?? null);
  const [opened, setOpened] = useState<Opened>(null);
  const [renaming, setRenaming] = useState(false);

  const parts = map?.parts ?? [];
  const decisions = groups ? currentDecisions(groups) : null;
  const docs = home ? projectDocs(home.docs) : null;
  const features = map ? [...map.features.values()].flat().length : null;
  const context: DecisionContext = { params, names, tracks, parts: map?.parts ?? null };
  const tones = map?.parts.map((part) => ({ kind: part.kind, tone: partTone(map.parts, part.id) }));

  return (
    <ScopePage>
      <PageColumns
        className="mt-0"
        panel={
          <PropertiesPanel>
            <PanelSection>
              <PropertyRow label="Kind" aside={map ? kinds(parts) : undefined}>
                <span>Project</span>
              </PropertyRow>
              <PropertyRow label="Parts">
                <div className="flex w-full flex-col">
                  {parts.map((part) => (
                    <PartResource key={part.id} part={part} parts={parts} params={params} />
                  ))}
                  {map && parts.length === 0 && <span className="pt-1 text-ink-3">None yet</span>}
                </div>
              </PropertyRow>
            </PanelSection>
            <PanelSection>
              <Counts>
                <CountLink {...projectLink(workspaceId, projectId, "parts")} n={parts.length} label={parts.length === 1 ? "part" : "parts"} />
                <CountLink {...projectLink(workspaceId, projectId, "parts")} n={features ?? 0} label={features === 1 ? "feature" : "features"} />
                <CountLink {...projectLink(workspaceId, projectId, "docs")} n={docs?.length ?? 0} label={docs?.length === 1 ? "doc" : "docs"} />
                <CountLink {...projectLink(workspaceId, projectId, "tracks")} n={tracks.filter(({ track }) => inFlight(track.state)).length} label="in flight" />
                <CountLink {...projectLink(workspaceId, projectId, "tracks")} n={tracks.filter(({ track }) => track.state === "done").length} label="done" />
                <CountLink {...projectLink(workspaceId, projectId, "decisions")} n={decisions?.length ?? 0} label={decisions?.length === 1 ? "decision" : "decisions"} />
              </Counts>
            </PanelSection>
          </PropertiesPanel>
        }
      >
        <div className="flex min-h-(--control-h-lg) items-center gap-3">
          <ProjectMark name={project?.name ?? ""} parts={tones} size="lg" />
          {project && <ProjectTitle workspaceId={workspaceId} projectId={projectId} name={project.name} editing={renaming} onEditing={setRenaming} />}
          {!renaming && (
            <div className="ml-auto flex shrink-0 items-center gap-2">
              <ProjectMenu onRename={() => setRenaming(true)} />
              <HeadActions secondary="part" opened={opened} onOpen={setOpened} />
            </div>
          )}
        </div>
        {opened === "part" && (
          <CreatePartForm
            {...params}
            onDone={(created) => {
              setOpened(null);
              if (created) reloadMap();
            }}
          />
        )}
        {opened === "track" && map && (
          <StartTrackForm
            {...params}
            parts={parts}
            onDone={(created) => {
              setOpened(null);
              if (created) reloadHome();
            }}
          />
        )}
        {(projectsFailed || homeFailed || mapFailed) && <Failed />}
        {project && (
          <Intro
            text={project.description}
            onSave={async (text) => {
              await describeProject(workspaceId, projectId, text);
              reloadProjects();
            }}
          />
        )}
        {home && (
          <InFlight
            tracks={tracks}
            all={tracks}
            params={params}
            map={map}
            names={names}
            questions={questions}
            showPart
            allLink={
              <Link {...projectLink(workspaceId, projectId, "tracks")} className={btnGhostSm}>
                All tracks
              </Link>
            }
            empty="Nothing is being changed. Start a track to change something."
          />
        )}
        {decisions && (
          <RecentDecisions
            decisions={decisions}
            context={context}
            showAbout
            allLink={
              <Link {...projectLink(workspaceId, projectId, "decisions")} className={btnGhostSm}>
                All decisions
              </Link>
            }
            empty="No decisions yet. Tracks record them as they answer questions."
          />
        )}
        {docs && (
          <RecentDocs
            docs={docs}
            target={(doc) => ({ to: "/w/$workspaceId/projects/$projectId/docs/$docId", params: { ...params, docId: doc.id } })}
            allLink={
              <Link {...projectLink(workspaceId, projectId, "docs")} className={btnGhostSm}>
                All docs
              </Link>
            }
            empty="No project docs yet."
          />
        )}
      </PageColumns>
    </ScopePage>
  );
}

function RecentDecisions({
  decisions,
  context,
  showAbout,
  allLink,
  empty,
}: {
  decisions: ReturnType<typeof currentDecisions>;
  context: DecisionContext;
  showAbout: boolean;
  allLink: ReactNode;
  empty: ReactNode;
}) {
  const recent = decisions.slice(0, 3);
  return (
    <Section
      id="recent-decisions"
      title="Recent decisions"
      count={decisions.length > recent.length ? `${recent.length} of ${decisions.length}` : decisions.length}
      end={decisions.length > 0 && allLink}
    >
      <ColumnList label="Recent decisions">
        {recent.length === 0 ? (
          <EmptyRow>{empty}</EmptyRow>
        ) : (
          recent.map(({ decision, subject }) => (
            <RecentDecisionRow key={decision.id} decision={decision} subject={subject} context={context} showAbout={showAbout} />
          ))
        )}
      </ColumnList>
    </Section>
  );
}

function RecentDocs({
  docs,
  target,
  allLink,
  empty,
}: {
  docs: HomeResponse["docs"];
  target: Parameters<typeof DocTiles>[0]["target"];
  allLink: ReactNode;
  empty: ReactNode;
}) {
  const recent = byRecent(docs).slice(0, 3);
  return (
    <Section
      id="recent-docs"
      title="Recent docs"
      count={docs.length > recent.length ? `${recent.length} of ${docs.length}` : docs.length}
      end={docs.length > 0 && allLink}
    >
      {recent.length === 0 ? (
        <ColumnList>
          <EmptyRow>{empty}</EmptyRow>
        </ColumnList>
      ) : (
        <DocTiles docs={recent} target={target} />
      )}
    </Section>
  );
}

/** A part's Overview. */
export function PartOverview({ workspaceId, projectId, partId }: Params & { partId: string }) {
  const params = { workspaceId, projectId };
  const { projects } = useProjects(workspaceId, projectId);
  const project = projects?.find((candidate) => candidate.id === projectId);
  const { home, failed: homeFailed, reload: reloadHome } = useHome(workspaceId, projectId);
  const { map, failed: mapFailed, reload: reloadMap } = useProjectMap(workspaceId, projectId);
  const { groups } = useDecisionGroups(workspaceId, projectId, project?.name, map, partId);
  const names = useMemberNames(workspaceId);
  const all = home?.tracks ?? [];
  const tracks = all.filter(({ track }) => track.part_id === partId);
  const questions = useRowFlags(workspaceId, home ? tracks : null);
  const [opened, setOpened] = useState<Opened>(null);

  const part = map?.parts.find((candidate) => candidate.id === partId);
  if (map && !part) return <PartNotFound params={params} />;
  const parts = map?.parts ?? [];
  const features = map?.features.get(partId) ?? [];
  const featureIds = new Set(features.map((feature) => feature.id));
  const docs = home && map ? partDocs(home.docs, partId, featureIds) : null;
  const decisions = groups ? currentDecisions(groups) : null;
  const context: DecisionContext = { params, names, tracks: all, parts: map?.parts ?? null };
  const tone = partTone(parts, partId);
  const changing = (featureId: string) =>
    (map?.changedBy.get(featureId) ?? [])
      .map((trackId) => all.find((summary) => summary.track.id === trackId))
      .filter((summary): summary is TrackSummary => Boolean(summary && inFlight(summary.track.state)));

  return (
    <ScopePage>
      <PageColumns
        className="mt-0"
        panel={
          <PropertiesPanel>
            <PanelSection>
              <PropertyRow label="Kind">
                <span>{part ? (part.kind === "app" ? "App" : part.kind === "service" ? "Service" : part.kind) : ""}</span>
              </PropertyRow>
              <PropertyRow label="Project">
                {project && (
                  <Link
                    {...projectLink(workspaceId, projectId, "overview")}
                    className="inline-flex h-6 max-w-full items-center gap-1.5 rounded-control border border-line px-1.5 font-medium text-ink transition-colors duration-(--t-fast) hover:bg-hover"
                  >
                    <ProjectMark name={project.name} parts={parts.map((one) => ({ kind: one.kind, tone: partTone(parts, one.id) }))} className="size-3.5 grid-cols-[repeat(2,3px)] grid-rows-[repeat(2,3px)] rounded-[3px]" />
                    <span className="truncate">{project.name}</span>
                  </Link>
                )}
              </PropertyRow>
              <PropertyRow label="Features">
                <div className="flex w-full flex-col">
                  {features.map((feature) => {
                    const by = changing(feature.id)[0];
                    return (
                      <Link
                        key={feature.id}
                        to="/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId"
                        params={{ ...params, partId, featureId: feature.id }}
                        className="-ml-1 flex h-7 w-full items-center gap-2 rounded-control px-1 font-medium text-ink transition-colors duration-(--t-fast) hover:bg-hover"
                      >
                        <Box aria-hidden className="size-(--icon-sm) shrink-0 text-ink-2" strokeWidth={1.75} />
                        <span className="min-w-0 truncate">{feature.name}</span>
                        {by && <StateIcon state={by.track.state} label={`Being changed by ${by.track.name}`} className="ml-auto" />}
                      </Link>
                    );
                  })}
                  {map && features.length === 0 && <span className="pt-1 text-ink-3">None yet</span>}
                </div>
              </PropertyRow>
            </PanelSection>
            <PanelSection>
              <Counts>
                <CountLink {...partLink(workspaceId, projectId, partId, "features")} n={features.length} label={features.length === 1 ? "feature" : "features"} />
                <CountLink {...partLink(workspaceId, projectId, partId, "docs")} n={docs?.length ?? 0} label={docs?.length === 1 ? "doc" : "docs"} />
                <CountLink {...partLink(workspaceId, projectId, partId, "decisions")} n={decisions?.length ?? 0} label={decisions?.length === 1 ? "decision" : "decisions"} />
                <CountLink {...partLink(workspaceId, projectId, partId, "tracks")} n={tracks.filter(({ track }) => inFlight(track.state)).length} label="in flight" />
                <CountLink {...partLink(workspaceId, projectId, partId, "tracks")} n={tracks.filter(({ track }) => track.state === "done").length} label="done" />
                <CountLink {...partLink(workspaceId, projectId, partId, "tracks")} n={tracks.filter(({ track }) => track.state === "dropped").length} label="dropped" />
              </Counts>
            </PanelSection>
          </PropertiesPanel>
        }
      >
        <PageHead
          mark={part ? <PartIcon name={part.name} kind={part.kind} tone={tone} size="lg" /> : undefined}
          title={part?.name ?? " "}
          actions={part && <HeadActions secondary="feature" opened={opened} onOpen={setOpened} />}
        />
        {opened === "feature" && (
          <CreateFeatureForm
            {...params}
            partId={partId}
            onDone={(created) => {
              setOpened(null);
              if (created) reloadMap();
            }}
          />
        )}
        {opened === "track" && (
          <StartTrackForm
            {...params}
            fixedPartId={partId}
            parts={parts}
            onDone={(created) => {
              setOpened(null);
              if (created) reloadHome();
            }}
          />
        )}
        {(homeFailed || mapFailed) && <Failed />}
        {part && (
          <Intro
            text={part.description}
            onSave={async (text) => {
              await describePart(workspaceId, projectId, partId, text);
              reloadMap();
            }}
          />
        )}
        {home && part && (
          <InFlight
            tracks={tracks}
            all={all}
            params={params}
            map={map}
            names={names}
            questions={questions}
            showPart={false}
            allLink={
              <Link {...partLink(workspaceId, projectId, partId, "tracks")} className={btnGhostSm}>
                All {part.name} tracks
              </Link>
            }
            empty={<>Nothing is being changed on {part.name}. Start a track to change it.</>}
          />
        )}
        {map && part && (
          <Section
            id="features"
            title="Features"
            count={features.length}
            end={
              <Link {...partLink(workspaceId, projectId, partId, "features")} className={btnGhostSm}>
                All features
              </Link>
            }
          >
            <ColumnList label="Features">
              {features.length === 0 ? (
                <EmptyRow>No features yet. A feature is something {part.name} can do; create one to say what.</EmptyRow>
              ) : (
                features.map((feature) => {
                  const by = changing(feature.id)[0];
                  return (
                    <li key={feature.id}>
                      <Link
                        to="/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId"
                        params={{ ...params, partId, featureId: feature.id }}
                        className={rowClass({ link: true })}
                      >
                        <RowLead>
                          <Box aria-hidden className="size-(--icon) text-ink-2" strokeWidth={1.5} />
                        </RowLead>
                        <span className="flex min-w-0 flex-1 items-center gap-2">
                          <span className="shrink-0 font-medium">{feature.name}</span>
                          {feature.description && <span className="min-w-0 truncate text-ink-3">{feature.description}</span>}
                        </span>
                        {by ? (
                          <span className="inline-flex shrink-0 items-center gap-1.5 text-meta text-ink-2" title={`Being changed by ${by.track.name}`}>
                            <StateIcon state={by.track.state} className="size-3" />
                            Being changed
                          </span>
                        ) : (
                          <span className="shrink-0 text-meta text-ink-3">Added {shortDate(feature.created_at)}</span>
                        )}
                      </Link>
                    </li>
                  );
                })
              )}
            </ColumnList>
          </Section>
        )}
        {decisions && part && (
          <RecentDecisions
            decisions={decisions}
            context={context}
            showAbout={false}
            allLink={
              <Link {...partLink(workspaceId, projectId, partId, "decisions")} className={btnGhostSm}>
                All decisions
              </Link>
            }
            empty={<>No decisions about {part.name} or its features yet.</>}
          />
        )}
        {docs && part && (
          <RecentDocs
            docs={docs}
            target={(doc) => ({ to: "/w/$workspaceId/projects/$projectId/parts/$partId/docs/$docId", params: { ...params, partId, docId: doc.id } })}
            allLink={
              <Link {...partLink(workspaceId, projectId, partId, "docs")} className={btnGhostSm}>
                All {part.name} docs
              </Link>
            }
            empty={<>No docs on {part.name} yet.</>}
          />
        )}
      </PageColumns>
    </ScopePage>
  );
}

/** A part that is not there (deleted, or of another project). */
export function PartNotFound({ params }: { params: Params }) {
  return (
    <ScopePage>
      <PageHead title="Part not found" />
      <p className="mt-2 text-em text-ink-2">
        The project has no such part, or it was deleted.{" "}
        <Link {...projectLink(params.workspaceId, params.projectId, "parts")} className="font-medium text-ink underline underline-offset-2">
          See the project&rsquo;s parts
        </Link>
      </p>
    </ScopePage>
  );
}
