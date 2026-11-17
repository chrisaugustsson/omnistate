import { useMemo, useState, type ReactNode } from "react";
import { Link, Navigate } from "@tanstack/react-router";
import { FileText, Palette, PenTool, Search } from "lucide-react";
import type { HomeDoc } from "@/api/generated";
import { matches } from "@/components/palette/search";
import { DocPage, type DocSearch } from "@/components/viewer/doc-page";
import { timeAgo } from "@/lib/format";
import { useHome } from "@/lib/use-home";
import { cn } from "@/lib/utils";
import { useProjectMap } from "./project-data";

// The docs of a scope (the project, or a part with its features): which docs
// those are, the pillars they are grouped by (decision U8: Spec, UI,
// Architecture, Database, Research), the small tiles an Overview shows, and
// the Docs tab: the list with a find field on the left, the open doc in the
// viewer on the right.

/** The pillars, in the order the Docs tab groups them. */
export const DOC_PILLARS = [
  { pillar: "spec", title: "Spec" },
  { pillar: "ui", title: "UI" },
  { pillar: "architecture", title: "Architecture" },
  { pillar: "db", title: "Database" },
  { pillar: "research", title: "Research" },
] as const;

export function pillarTitle(pillar: string): string {
  return DOC_PILLARS.find((candidate) => candidate.pillar === pillar)?.title ?? pillar.charAt(0).toUpperCase() + pillar.slice(1);
}

/** The project's own docs: those on no track, part or feature. */
export function projectDocs(docs: readonly HomeDoc[]): HomeDoc[] {
  return docs.filter((doc) => !doc.track_id && !doc.part_id && !doc.feature_id);
}

/** A part's docs: those on the part itself and those on its features (`featureIds`); not its tracks'. */
export function partDocs(docs: readonly HomeDoc[], partId: string, featureIds: ReadonlySet<string>): HomeDoc[] {
  return docs.filter((doc) => !doc.track_id && (doc.feature_id ? featureIds.has(doc.feature_id) : doc.part_id === partId));
}

const updated = (doc: HomeDoc) => Date.parse(doc.updated_at ?? doc.created_at) || 0;

/** The most recently updated first, ties by title. */
export function byRecent(docs: readonly HomeDoc[]): HomeDoc[] {
  return [...docs].sort((a, b) => updated(b) - updated(a) || a.title.localeCompare(b.title));
}

export function DocIcon({ kind, className }: { kind: string; className?: string }) {
  const Icon = kind === "design" ? PenTool : kind === "design-system" ? Palette : FileText;
  return <Icon aria-hidden strokeWidth={1.75} className={cn("size-(--icon-sm) shrink-0 text-ink-2", className)} />;
}

/** Where a doc of the list opens: a router link's `to` and `params`. */
export type DocTarget = (doc: HomeDoc) => { to: string; params: Record<string, string> };

/** An Overview's recent docs as small tiles: icon, title, pillar and when it changed. */
export function DocTiles({ docs, target }: { docs: readonly HomeDoc[]; target: DocTarget }) {
  return (
    <ul className="grid grid-cols-1 gap-3 sm:grid-cols-3">
      {docs.map((doc) => (
        <li key={doc.id} className="min-w-0">
          <Link
            {...(target(doc) as { to: "/w/$workspaceId/projects/$projectId/docs/$docId"; params: { workspaceId: string; projectId: string; docId: string } })}
            className="flex min-w-0 items-start gap-2.5 rounded-card border border-line bg-sheet p-3 transition-colors duration-(--t-fast) hover:border-line-strong hover:bg-frame focus-visible:shadow-focus focus-visible:outline-none"
          >
            <DocIcon kind={doc.kind} className="mt-px" />
            <span className="flex min-w-0 flex-col gap-0.5">
              <span className="truncate font-medium">{doc.title}</span>
              <span className="truncate text-meta text-ink-3">
                {pillarTitle(doc.pillar)} · {timeAgo(doc.updated_at ?? doc.created_at)}
              </span>
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}

/** The doc list: a find field, then the docs grouped by pillar with a count each, the open one lit. */
function DocList({ docs, current, target, about }: { docs: readonly HomeDoc[]; current?: string; target: DocTarget; about?: (doc: HomeDoc) => string | undefined }) {
  const [query, setQuery] = useState("");
  const shown = useMemo(() => (query.trim() ? docs.filter((doc) => matches(query, doc.title)) : docs), [docs, query]);
  const groups = DOC_PILLARS.map(({ pillar, title }) => ({
    pillar,
    title,
    docs: shown.filter((doc) => doc.pillar === pillar).sort((a, b) => a.title.localeCompare(b.title)),
  }));
  const other = shown.filter((doc) => !DOC_PILLARS.some(({ pillar }) => pillar === doc.pillar));
  if (other.length > 0) groups.push({ pillar: "other" as never, title: "Other" as never, docs: other });
  return (
    <nav aria-label="Docs" className="flex flex-col gap-px">
      <label className="mb-2 flex h-(--control-h) items-center gap-2 rounded-control border border-line-strong bg-sheet px-2 text-ink-3 focus-within:border-brand focus-within:shadow-focus">
        <Search aria-hidden className="size-(--icon-sm) shrink-0" />
        <input
          type="search"
          aria-label="Find a doc"
          placeholder="Find a doc"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          className="min-w-0 flex-1 border-0 bg-transparent text-ink outline-none placeholder:text-ink-3"
        />
      </label>
      {groups
        .filter((group) => group.docs.length > 0)
        .map((group) => (
          <section key={group.pillar} aria-label={group.title} className="mt-3 flex flex-col gap-px first-of-type:mt-0">
            <h3 className="flex h-7 items-center px-2 text-meta font-medium text-ink-3">
              {group.title}
              <span className="ml-auto tabular-nums">{group.docs.length}</span>
            </h3>
            <ul className="flex flex-col gap-px">
              {group.docs.map((doc) => {
                const note = about?.(doc);
                return (
                  <li key={doc.id}>
                    <Link
                      {...(target(doc) as { to: "/w/$workspaceId/projects/$projectId/docs/$docId"; params: { workspaceId: string; projectId: string; docId: string } })}
                      aria-current={doc.id === current ? "page" : undefined}
                      className="flex h-[30px] items-center gap-2 rounded-control px-2 font-medium whitespace-nowrap text-ink transition-colors duration-(--t-fast) hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none aria-[current=page]:bg-selected"
                    >
                      <DocIcon kind={doc.kind} />
                      <span className="min-w-0 truncate">{doc.title}</span>
                      {note && <span className="ml-auto truncate pl-2 text-meta font-normal text-ink-3">{note}</span>}
                    </Link>
                  </li>
                );
              })}
            </ul>
          </section>
        ))}
      {docs.length > 0 && shown.length === 0 && <p className="px-2 py-1.5 text-meta text-ink-3">No doc title holds “{query.trim()}”.</p>}
    </nav>
  );
}

/**
 * A Docs tab: the list on the left (248px), and on the right the open doc
 * (`children`, the viewer) or, with no doc open, `empty`. It fills the page
 * under the bar; each side scrolls on its own.
 */
export function DocsTab({
  docs,
  current,
  target,
  about,
  failed,
  children,
}: {
  docs: readonly HomeDoc[] | null;
  current?: string;
  target: DocTarget;
  /** A quiet note after a doc's title (the feature a part's doc is about). */
  about?: (doc: HomeDoc) => string | undefined;
  failed?: boolean;
  children: ReactNode;
}) {
  return (
    <div className="flex h-full min-h-0 flex-col md:flex-row">
      <aside aria-label="Doc list" className="shrink-0 border-line px-4 py-5 md:w-[calc(var(--doclist-w)+32px)] md:overflow-y-auto md:border-r">
        {failed && <p role="alert" className="mb-2 text-danger-ink">The docs could not be loaded. Try again in a moment.</p>}
        {docs === null ? (
          <span aria-hidden className="block h-(--control-h) animate-pulse rounded-control bg-hover" />
        ) : (
          <DocList docs={docs} current={current} target={target} about={about} />
        )}
      </aside>
      <div className="min-h-0 min-w-0 flex-1 md:overflow-y-auto">{children}</div>
    </div>
  );
}

/** A Docs tab with no doc open: the most recently updated doc opens; with none, `empty` says so. */
export function OpenLatest({ docs, target, empty }: { docs: readonly HomeDoc[] | null; target: DocTarget; empty: ReactNode }) {
  if (docs === null) return null;
  const latest = byRecent(docs)[0];
  if (latest) return <Navigate {...(target(latest) as { to: "/w/$workspaceId/projects/$projectId/docs/$docId"; params: { workspaceId: string; projectId: string; docId: string } })} replace />;
  return (
    <div className="grid h-full min-h-60 place-items-center p-8 text-center">
      <div className="max-w-sm">
        <FileText aria-hidden strokeWidth={1.5} className="mx-auto mb-3 size-6 text-faint" />
        <p className="text-ink-2">{empty}</p>
      </div>
    </div>
  );
}

/**
 * The Docs tab of the project (no `partId`: its own docs) or of a part (the
 * part's and its features' docs), with `docId` open in the viewer; without
 * one, the latest doc opens.
 */
export function ScopeDocs({
  workspaceId,
  projectId,
  partId,
  docId,
  search,
}: {
  workspaceId: string;
  projectId: string;
  partId?: string;
  docId?: string;
  search?: DocSearch;
}) {
  const { home, failed } = useHome(workspaceId, projectId);
  const { map } = useProjectMap(workspaceId, projectId);
  const features = partId ? (map?.features.get(partId) ?? null) : null;
  const docs = useMemo(() => {
    if (!home) return null;
    if (!partId) return projectDocs(home.docs);
    if (!features) return null;
    return partDocs(home.docs, partId, new Set(features.map((feature) => feature.id)));
  }, [home, partId, features]);
  const params = { workspaceId, projectId };
  const target: DocTarget = partId
    ? (doc) => ({ to: "/w/$workspaceId/projects/$projectId/parts/$partId/docs/$docId", params: { ...params, partId, docId: doc.id } })
    : (doc) => ({ to: "/w/$workspaceId/projects/$projectId/docs/$docId", params: { ...params, docId: doc.id } });
  const about = features ? (doc: HomeDoc) => features.find((feature) => feature.id === doc.feature_id)?.name : undefined;
  const scope = partId ? (map?.parts.find((part) => part.id === partId)?.name ?? "the part") : "the project";
  return (
    <DocsTab docs={docs} current={docId} target={target} about={about} failed={failed}>
      {docId ? (
        <DocPage workspaceId={workspaceId} projectId={projectId} docId={docId} search={search ?? {}} />
      ) : (
        <OpenLatest
          docs={docs}
          target={target}
          empty={<>No docs on {scope} yet. Agents write them as they work, and each one shows up here under its pillar.</>}
        />
      )}
    </DocsTab>
  );
}
