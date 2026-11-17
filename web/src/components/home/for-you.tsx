import { Link } from "@tanstack/react-router";
import { CircleCheck } from "lucide-react";
import type { HomeResponse, MemberBody, PartBody, ProjectBody, TrackSummary } from "@/api/generated";
import { Avatar } from "@/components/shell/avatar";
import { Page, PageColumns } from "@/components/shell/page";
import { PartIcon, ProjectMark } from "@/components/shell/part-icon";
import { PanelSection, PropertiesPanel } from "@/components/shell/properties-panel";
import { homeLink, projectLink, partLink, docLink } from "@/components/shell/scope";
import { StateIcon, inFlight, stateLabel } from "@/components/shell/state-icon";
import { partTone, partTones, personName } from "@/lib/identity";
import { cn } from "@/lib/utils";
import {
  STATE_ORDER,
  activityOf,
  age,
  firstLine,
  greeting,
  myTracks,
  plural,
  splitText,
  waitingGroups,
  whoOf,
  type WaitingGroup,
  type WaitingItem,
} from "./model";
import { Band, ColumnList, EmptyRow, SectionHead, TrackRow, buttonClass, trackLink } from "./rows";
import { findPart, useHomeData, useTrackFlags, type TrackFlags } from "./use-home-data";

// Home's For you (tasks/ui-redesign/v2/home.html): a greeting with the
// reader's name and the time of day, one line on what waits, what waits on
// the reader grouped by track, and the reader's tracks. Beside it, the
// workspace's projects and its people.

/** How many of the reader's tracks For you lists; My tracks has them all. */
const MOST_TRACKS = 6;

export function ForYou({ workspaceId }: { workspaceId: string }) {
  const { user, workspace, projects, homes, parts, me, members, failed } = useHomeData(workspaceId);
  const groups = waitingGroups(homes, me);
  const mine = myTracks(homes, me);
  const flags = useTrackFlags(workspaceId, mine?.map(({ track }) => track) ?? null);
  const shown = mine
    ?.filter(({ track }) => track.state !== "dropped")
    .sort((a, b) => STATE_ORDER.indexOf(a.track.state as never) - STATE_ORDER.indexOf(b.track.state as never) || activityOf(b) - activityOf(a))
    .slice(0, MOST_TRACKS);
  const now = new Date();

  return (
    <Page>
      <PageColumns
        className="mt-0"
        panel={
          <WorkspacePanel
            workspaceName={workspace.name}
            workspaceId={workspaceId}
            projects={projects}
            homes={homes}
            parts={parts}
            members={members}
            me={me}
          />
        }
      >
        <div className="flex min-h-(--control-h-lg) items-center gap-3">
          <h1 className="min-w-0 text-greeting font-semibold">
            {greeting(now)}, {personName(user.email)}
          </h1>
          <span className="ml-auto text-meta whitespace-nowrap text-ink-3">
            {now.toLocaleDateString("en-GB", { weekday: "long", day: "numeric", month: "long" })}
          </span>
        </div>
        {failed && <p className="mt-2 text-danger-ink">Home could not be loaded. Try again in a moment.</p>}
        {groups && mine && <Summary workspaceId={workspaceId} groups={groups} mine={mine} flags={flags} />}

        <section aria-labelledby="h-waiting" className="mt-10">
          <SectionHead id="h-waiting" title="Waiting on you" count={groups ? groups.reduce((sum, group) => sum + group.items.length, 0) : undefined}>
            <Link {...homeLink(workspaceId, "inbox")} className={buttonClass({ ghost: true, sm: true })}>
              Open inbox
            </Link>
          </SectionHead>
          {groups && (
            <ColumnList>
              {groups.length === 0 ? (
                <EmptyRow icon={<CircleCheck aria-hidden className="mx-0.5 size-(--icon) text-faint" strokeWidth={2} />}>
                  You&rsquo;re all caught up. When an agent replies to a comment on one of your tracks, it shows up here.
                </EmptyRow>
              ) : (
                groups.map((group) => (
                  <WaitingGroupRows key={group.key} workspaceId={workspaceId} group={group} parts={parts} members={members} me={me} />
                ))
              )}
            </ColumnList>
          )}
        </section>

        <section aria-labelledby="h-mine" className="mt-8">
          <SectionHead id="h-mine" title="Your tracks" count={mine?.filter(({ track }) => inFlight(track.state)).length}>
            <Link {...homeLink(workspaceId, "my-tracks")} className={buttonClass({ ghost: true, sm: true })}>
              All my tracks
            </Link>
          </SectionHead>
          {shown && (
            <ColumnList label="Your tracks">
              {shown.length === 0 ? (
                <EmptyRow>You own no tracks yet. Start one on a part&rsquo;s Tracks.</EmptyRow>
              ) : (
                shown.map((summary) => {
                  const home = homes?.get(summary.track.project_id);
                  const split = splitText(summary, home);
                  return (
                    <TrackRow
                      key={summary.track.id}
                      two
                      workspaceId={workspaceId}
                      summary={summary}
                      sub={[stateLabel(summary.track.state), split].filter(Boolean).join(" · ")}
                      flags={flags.get(summary.track.id)}
                      part={findPart(parts, summary.track.project_id, summary.track.part_id)}
                      parts={parts?.get(summary.track.project_id)}
                    />
                  );
                })
              )}
            </ColumnList>
          )}
        </section>
      </PageColumns>
    </Page>
  );
}

/** The line under the greeting: how much waits and where, and what the reader's tracks need. */
function Summary({
  workspaceId,
  groups,
  mine,
  flags,
}: {
  workspaceId: string;
  groups: WaitingGroup[];
  mine: TrackSummary[];
  flags: ReadonlyMap<string, TrackFlags>;
}) {
  const waiting = groups.reduce((sum, group) => sum + group.items.length, 0);
  const live = mine.filter(({ track }) => inFlight(track.state));
  const asking = live
    .map((summary) => ({ summary, questions: flags.get(summary.track.id)?.questions ?? 0 }))
    .filter(({ questions }) => questions > 0)
    .sort((a, b) => b.questions - a.questions)[0];
  const name = (summary: TrackSummary) => (
    <Link {...trackLink(workspaceId, summary.track)} className="font-medium text-ink hover:underline hover:decoration-line-strong hover:underline-offset-2">
      {summary.track.name}
    </Link>
  );
  const only = groups.length === 1 ? groups[0].track : undefined;
  return (
    <p className="mt-2 max-w-[640px] text-em leading-(--lh-prose) text-ink-2">
      {waiting === 0 ? (
        <>Nothing waits on you. </>
      ) : only ? (
        <>
          {plural(waiting, "comment waits", "comments wait")} on you, {waiting === 1 ? "on" : "all on"} {name(only)}.{" "}
        </>
      ) : (
        <>
          {plural(waiting, "comment waits", "comments wait")} on you, on {plural(groups.length, "track", "tracks")}.{" "}
        </>
      )}
      {asking ? (
        <>
          {name(asking.summary)} has {plural(asking.questions, "behaviour question", "behaviour questions")} to answer.
        </>
      ) : live.length > 0 ? (
        <>You have {plural(live.length, "track", "tracks")} in flight.</>
      ) : mine.length === 0 ? (
        <>You own no tracks yet.</>
      ) : (
        <>None of your tracks is in flight.</>
      )}
    </p>
  );
}

/** One track's band and the comments that wait on it. */
function WaitingGroupRows({
  workspaceId,
  group,
  parts,
  members,
  me,
}: {
  workspaceId: string;
  group: WaitingGroup;
  parts: ReadonlyMap<string, PartBody[]> | null;
  members: MemberBody[] | null;
  me: string | null;
}) {
  const track = group.track?.track;
  const part = track ? findPart(parts, group.projectId, track.part_id) : undefined;
  return (
    <>
      <Band>
        {track && <StateIcon state={track.state} />}
        {track && (
          <Link
            {...trackLink(workspaceId, track)}
            className="-mx-1.5 inline-flex h-6 min-w-0 items-center truncate rounded-badge px-1.5 text-ink transition-colors hover:bg-hover"
          >
            {track.name}
          </Link>
        )}
        {track && (
          <span className="text-meta font-normal whitespace-nowrap text-ink-3">
            {[part?.name ?? group.track?.part_name, stateLabel(track.state).toLowerCase()].filter(Boolean).join(" · ")}
          </span>
        )}
        <span className="font-medium text-ink-3 tabular-nums">{group.items.length}</span>
      </Band>
      {group.items.map((item) => (
        <WaitingRow key={item.comment.id} workspaceId={workspaceId} item={item} members={members} me={me} />
      ))}
    </>
  );
}

/** A comment that waits: who replied, what they said, on which doc, how long ago; Reply opens it in the Inbox. */
function WaitingRow({ workspaceId, item, members, me }: { workspaceId: string; item: WaitingItem; members: MemberBody[] | null; me: string | null }) {
  const who = whoOf(item.reply.author, members, me);
  const doc = item.doc ?? { id: item.comment.doc_id, track_id: item.comment.track_id };
  return (
    <div className="group relative flex items-start gap-3 border-t border-line py-2.5 transition-colors duration-(--t-fast) ease-out-quint [div[data-band]+&]:border-t-0 hover:bg-frame hover:shadow-[-8px_0_0_var(--frame),8px_0_0_var(--frame)]">
      <Avatar name={who.avatarName ?? who.name} agent={who.agent} tintKey={who.tintKey} title={who.name} />
      <Link
        {...docLink(workspaceId, item.projectId, doc)}
        search={{ comment: item.comment.id }}
        className="flex min-w-0 flex-1 flex-col gap-0.5 text-ink after:absolute after:inset-0 focus-visible:outline-none"
      >
        <span className="max-w-full min-w-0 truncate font-medium">{firstLine(item.reply.text)}</span>
        <span className="max-w-full min-w-0 truncate text-ink-3">
          {who.name} replied to your comment{item.doc ? ` · ${item.doc.title}` : ""}
        </span>
      </Link>
      <span className="w-16 shrink-0 text-right text-meta whitespace-nowrap text-ink-3 tabular-nums transition-opacity group-focus-within:opacity-0 group-hover:opacity-0">
        {age(item.at)}
      </span>
      <Link
        to="/w/$workspaceId/inbox"
        params={{ workspaceId }}
        search={{ comment: item.comment.id }}
        className={cn(
          buttonClass({ sm: true }),
          "pointer-events-none absolute top-1/2 right-0 z-10 -translate-y-1/2 opacity-0 group-focus-within:pointer-events-auto group-focus-within:opacity-100 group-hover:pointer-events-auto group-hover:opacity-100",
        )}
      >
        Reply
      </Link>
    </div>
  );
}

/** The panel beside For you: the workspace's projects with their parts, and its people with what they have in flight. */
function WorkspacePanel({
  workspaceName,
  workspaceId,
  projects,
  homes,
  parts,
  members,
  me,
}: {
  workspaceName: string;
  workspaceId: string;
  projects: readonly ProjectBody[] | null;
  homes: ReadonlyMap<string, HomeResponse> | null;
  parts: ReadonlyMap<string, PartBody[]> | null;
  members: MemberBody[] | null;
  me: string | null;
}) {
  const live = homes ? [...homes.values()].flatMap((home) => (home.tracks ?? []).filter(({ track }) => inFlight(track.state))) : null;
  const resource =
    "flex h-[30px] items-center gap-2 rounded-control px-1 font-medium text-ink transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none";
  const meta = "ml-auto shrink-0 pl-2 text-meta font-normal whitespace-nowrap text-ink-3";
  return (
    <PropertiesPanel label="Your workspace">
      <PanelSection title="Your projects">
        {projects?.length === 0 && <p className="px-1 text-meta text-ink-3">No projects yet.</p>}
        {projects?.map((project) => {
          const projectParts = parts?.get(project.id);
          const tones = projectParts ? partTones(projectParts) : null;
          const marks = projectParts
            ? [...projectParts]
                .sort((a, b) => (tones?.get(a.id) ?? 0) - (tones?.get(b.id) ?? 0))
                .map((part) => ({ kind: part.kind, tone: tones?.get(part.id) ?? 1 }))
            : undefined;
          const flying = live?.filter(({ track }) => track.project_id === project.id).length;
          return (
            <div key={project.id}>
              <Link {...projectLink(workspaceId, project.id, "overview")} className={resource}>
                <ProjectMark name={project.name} parts={marks} />
                <span className="min-w-0 truncate">{project.name}</span>
                <span className={meta}>{flying !== undefined ? `${flying} in flight` : ""}</span>
              </Link>
              {projectParts?.map((part) => (
                <Link key={part.id} {...partLink(workspaceId, project.id, part.id, "overview")} className={cn(resource, "ml-5")}>
                  <PartIcon name={part.name} kind={part.kind} tone={partTone(projectParts, part.id)} size="sm" />
                  <span className="min-w-0 truncate">{part.name}</span>
                  <span className={meta}>{part.kind}</span>
                </Link>
              ))}
            </div>
          );
        })}
      </PanelSection>
      <PanelSection title={`People in ${workspaceName}`}>
        {members?.map((member) => {
          const theirs = live?.filter(({ track }) => track.owner_user_id === member.user_id) ?? [];
          const counts = (["planning", "building"] as const)
            .map((state) => [state, theirs.filter(({ track }) => track.state === state).length] as const)
            .filter(([, count]) => count > 0)
            .map(([state, count]) => `${count} ${state}`);
          const text = [member.user_id === me ? "you" : "", counts.join(", ") || (member.user_id === me ? "" : "nothing in flight")]
            .filter(Boolean)
            .join(" · ");
          return (
            <div key={member.membership_id} className="flex h-[30px] items-center gap-2 px-1 font-medium">
              <Avatar name={personName(member.email)} tintKey={member.user_id} />
              <span className="min-w-0 truncate">{personName(member.email)}</span>
              <span className={meta}>{text || "nothing in flight"}</span>
            </div>
          );
        })}
      </PanelSection>
    </PropertiesPanel>
  );
}
