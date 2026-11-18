import { Link } from "@tanstack/react-router";
import type { HomeDoc } from "@/api/generated";
import { OpenDoc, useDocListToggle } from "@/components/project/docs-pane";
import type { DocSearch } from "@/components/viewer/doc-page";
import { cn } from "@/lib/utils";
import { DocGlyph } from "./feature-parts";
import { useFeature } from "./feature-data";
import { EmptyTab } from "./ui";

// A feature's docs, as its Docs and Design tabs show them: the list grouped by
// pillar on the left, the open doc on the right: page text with its title as
// a heading, or a design's canvas filling the room under the tabs (the layout
// then spans the window, featureCanvas).

const PILLARS = [
  { pillar: "spec", title: "Spec" },
  { pillar: "ui", title: "UI" },
  { pillar: "architecture", title: "Architecture" },
  { pillar: "db", title: "Database" },
  { pillar: "research", title: "Research" },
];

const item =
  "flex h-[30px] w-full items-center gap-2 rounded-control px-2 text-left font-medium whitespace-nowrap text-ink transition-colors duration-(--t-fast) hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none aria-[current=page]:bg-selected aria-pressed:bg-selected";

/** The doc a Docs or Design tab opens: `current`, or the first of the list when none is named. */
export function openFeatureDoc(docs: HomeDoc[], current: string | undefined): HomeDoc | undefined {
  const ordered = PILLARS.flatMap(({ pillar }) =>
    docs.filter((doc) => doc.pillar === pillar).sort((a, b) => a.title.toLowerCase().localeCompare(b.title.toLowerCase())),
  );
  return ordered.find((doc) => doc.id === current) ?? (current ? undefined : ordered[0]);
}

export function FeatureDocs({
  docs,
  current,
  search,
  onPick,
  empty,
}: {
  docs: HomeDoc[];
  /** The open doc; the first of the list when not given. */
  current?: string;
  search: DocSearch;
  /** Given, the list picks the open doc in place; otherwise each doc is a link to its own address. */
  onPick?: (docId: string) => void;
  empty: { title: string; body: string };
}) {
  const { workspaceId, projectId, partId, featureId, home } = useFeature();
  const list = useDocListToggle();
  if (!home) return <div aria-hidden className="h-40 animate-pulse rounded-card bg-frame" />;
  const groups = PILLARS.map(({ pillar, title }) => ({
    title,
    docs: docs.filter((doc) => doc.pillar === pillar).sort((a, b) => a.title.toLowerCase().localeCompare(b.title.toLowerCase())),
  })).filter((group) => group.docs.length > 0);
  const ordered = groups.flatMap((group) => group.docs);
  const open = ordered.find((doc) => doc.id === current) ?? (current ? undefined : ordered[0]);
  if (ordered.length === 0) return <EmptyTab title={empty.title}>{empty.body}</EmptyTab>;
  const canvas = open?.kind === "design";
  return (
    <div className={cn("flex flex-col gap-8 md:flex-row", canvas ? "min-h-0 flex-1 md:gap-6" : "items-start md:gap-12")}>
      <nav
        aria-label="Doc list"
        hidden={canvas && !list.shown}
        className={cn("shrink-0 md:w-(--doclist-w)", canvas ? "md:overflow-y-auto" : "md:sticky md:top-6")}
      >
        {groups.map((group) => (
          <section key={group.title} aria-label={group.title} className="mt-2 first:mt-0">
            <h3 className="flex h-7 items-center px-2 text-meta font-medium text-ink-3">
              {group.title}
              <span className="ml-auto tabular-nums">{group.docs.length}</span>
            </h3>
            <ul className="flex flex-col gap-px">
              {group.docs.map((doc) => (
                <li key={doc.id}>
                  {onPick ? (
                    <button type="button" aria-pressed={doc.id === open?.id} onClick={() => onPick(doc.id)} className={item}>
                      <DocGlyph kind={doc.kind} />
                      <span className="min-w-0 truncate">{doc.title}</span>
                    </button>
                  ) : (
                    <Link
                      to="/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/docs/$docId"
                      params={{ workspaceId, projectId, partId, featureId, docId: doc.id }}
                      aria-current={doc.id === open?.id ? "page" : undefined}
                      activeProps={{}}
                      className={item}
                    >
                      <DocGlyph kind={doc.kind} />
                      <span className="min-w-0 truncate">{doc.title}</span>
                    </Link>
                  )}
                </li>
              ))}
            </ul>
          </section>
        ))}
      </nav>
      <div className={cn("min-w-0 flex-1", canvas && "flex min-h-0 flex-col")}>
        {open ? (
          <OpenDoc workspaceId={workspaceId} projectId={projectId} docId={open.id} doc={open} search={search} heading="h2" list={list} />
        ) : (
          <p className="text-ink-2">This doc is not on the feature. Pick one from the list.</p>
        )}
      </div>
    </div>
  );
}
