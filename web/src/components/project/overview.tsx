import type { DocBody, HomeResponse } from "@/api/generated";
import {
  ChangesetRow,
  DocRow,
  Empty,
  List,
  PILLARS,
  Section,
  ThreadRow,
  ViewAll,
  WaitingList,
  featureNames,
  sectionLink,
  waitingCount,
  waitingGroups,
  type FeatureNames,
} from "@/components/project/items";

// The parts of an overview: what waits for the reviewer, and the docs, change
// sets and threads. The project's overview and a feature's overview show the
// same parts, the project's over everything (each feature's items tagged with
// the feature's name), a feature's over its own `home` (see `scopedHome`).

type Params = { workspaceId: string; projectId: string };

export function OverviewSkeleton() {
  return (
    <div className="mt-10 space-y-3" aria-hidden>
      {[0, 1, 2].map((row) => (
        <div key={row} className="h-14 animate-pulse rounded-xl bg-muted" />
      ))}
    </div>
  );
}

/** The parts of one overview, over `home`; `featureId` is the feature that `home` is scoped to, if any. */
export function OverviewSections({ home, params, featureId }: { home: HomeResponse; params: Params; featureId?: string }) {
  const names = featureId ? undefined : featureNames(home);
  const owner = featureId ? "The feature" : "The project";
  return (
    <div className="space-y-12">
      <Waiting home={home} params={params} names={names} inbox={!featureId} />
      <div className="grid gap-12 lg:grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)] lg:gap-10">
        <Docs docs={home.docs} params={params} featureId={featureId} names={names} owner={owner} />
        <div className="space-y-12">
          <ChangeSets home={home} params={params} featureId={featureId} names={names} owner={owner} />
          <Threads home={home} params={params} featureId={featureId} names={names} owner={owner} />
        </div>
      </div>
    </div>
  );
}

function Waiting({
  home,
  params,
  names,
  inbox,
}: {
  home: HomeResponse;
  params: Params;
  names?: FeatureNames;
  /** Whether the inbox, which holds everything that waits, is the place to view all. */
  inbox: boolean;
}) {
  const count = waitingCount(home);
  return (
    <Section
      id="waiting"
      title="Waiting for you"
      count={count}
      more={inbox && <ViewAll to="/w/$workspaceId/projects/$projectId/inbox" params={params} />}
    >
      {!count && <Empty>Nothing waits for you.</Empty>}
      <div className="grid gap-6 md:grid-cols-2">
        {waitingGroups(home, params, names).map((group) => (
          <WaitingList key={group.kind} group={group} />
        ))}
      </div>
    </Section>
  );
}

type PartProps = { params: Params; featureId?: string; names?: FeatureNames; owner: string };

function Docs({ docs, params, featureId, names, owner }: PartProps & { docs: DocBody[] }) {
  return (
    <Section
      id="docs"
      title="Docs"
      count={docs.length}
      more={<ViewAll {...sectionLink("docs", params, featureId)} />}
    >
      {docs.length === 0 && <Empty>{owner} has no docs yet.</Empty>}
      <div className="space-y-6">
        {PILLARS.map(({ pillar, title, blurb }) => {
          const inPillar = docs.filter((doc) => doc.pillar === pillar);
          if (!inPillar.length) return null;
          const id = `docs-${pillar}`;
          return (
            <section key={pillar} aria-labelledby={id} className="space-y-2">
              <div className="flex items-baseline gap-2">
                <h3 id={id} className="text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
                  {title}
                </h3>
                <span className="text-xs text-muted-foreground/70">{blurb}</span>
              </div>
              <List>
                {inPillar.map((doc) => (
                  <DocRow key={doc.id} doc={doc} params={params} names={names} />
                ))}
              </List>
            </section>
          );
        })}
      </div>
    </Section>
  );
}

function ChangeSets({ home, params, featureId, names, owner }: PartProps & { home: HomeResponse }) {
  return (
    <Section
      id="changesets"
      title="Change sets"
      count={home.changesets.length}
      more={<ViewAll {...sectionLink("changesets", params, featureId)} />}
    >
      {home.changesets.length === 0 && <Empty>{owner} has no change sets yet.</Empty>}
      <List className="empty:hidden">
        {home.changesets.map((changeset) => (
          <ChangesetRow key={changeset.id} changeset={changeset} params={params} names={names} />
        ))}
      </List>
    </Section>
  );
}

function Threads({ home, params, featureId, names, owner }: PartProps & { home: HomeResponse }) {
  return (
    <Section
      id="threads"
      title="Threads"
      count={home.threads.length}
      more={<ViewAll {...sectionLink("threads", params, featureId)} />}
    >
      {home.threads.length === 0 && <Empty>{owner} has no threads yet.</Empty>}
      <List className="empty:hidden">
        {home.threads.map((thread) => (
          <ThreadRow key={thread.id} thread={thread} params={params} names={names} />
        ))}
      </List>
    </Section>
  );
}
