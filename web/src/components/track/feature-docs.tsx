import { Link } from "@tanstack/react-router";
import type { HomeDoc } from "@/api/generated";
import { DocPage, type DocSearch } from "@/components/viewer/doc-page";
import { cn } from "@/lib/utils";
import { DocGlyph } from "./feature-parts";
import { useFeature } from "./feature-data";
import { EmptyTab } from "./ui";

// A feature's docs, as its Docs and Design tabs show them: the list grouped by
// pillar on the left, the open doc in the viewer on the right, filling the room
// under the tabs.

const PILLARS = [
  { pillar: "spec", title: "Spec" },
  { pillar: "ui", title: "UI" },
  { pillar: "architecture", title: "Architecture" },
  { pillar: "db", title: "Database" },
  { pillar: "research", title: "Research" },
];

const item =
  "flex h-[30px] w-full items-center gap-2 rounded-control px-2 text-left font-medium whitespace-nowrap text-ink transition-colors duration-(--t-fast) hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none aria-[current=page]:bg-selected aria-pressed:bg-selected";

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
  if (!home) return <div aria-hidden className="h-40 animate-pulse rounded-card bg-frame" />;
  const groups = PILLARS.map(({ pillar, title }) => ({
    title,
    docs: docs.filter((doc) => doc.pillar === pillar).sort((a, b) => a.title.toLowerCase().localeCompare(b.title.toLowerCase())),
  })).filter((group) => group.docs.length > 0);
  const ordered = groups.flatMap((group) => group.docs);
  const open = ordered.find((doc) => doc.id === current) ?? (current ? undefined : ordered[0]);
  if (ordered.length === 0) return <EmptyTab title={empty.title}>{empty.body}</EmptyTab>;
  return (
    <div className="flex min-h-[560px] flex-1 flex-col overflow-hidden rounded-card border border-line md:flex-row">
      <nav aria-label="Doc list" className="shrink-0 border-b border-line p-2 md:w-(--doclist-w) md:overflow-y-auto md:border-r md:border-b-0">
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
      <div className={cn("relative min-h-[560px] min-w-0 flex-1")}>
        <div className="absolute inset-0 overflow-auto">
          {open ? (
            <DocPage key={open.id} workspaceId={workspaceId} projectId={projectId} docId={open.id} search={search} />
          ) : (
            <p className="p-6 text-ink-2">This doc is not on the feature. Pick one from the list.</p>
          )}
        </div>
      </div>
    </div>
  );
}
