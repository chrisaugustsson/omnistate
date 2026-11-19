import { useMemo, useState, type ReactNode } from "react";
import { Link, Navigate } from "@tanstack/react-router";
import { ChevronLeft, FileText, Palette, PanelLeftClose, PanelLeftOpen, PenTool, Search } from "lucide-react";
import type { HomeDoc } from "@/api/generated";
import { matches } from "@/components/palette/search";
import { partLink, projectLink } from "@/components/shell/scope";
import { useIsPhone } from "@/components/shell/shell-context";
import { DocByline } from "@/components/viewer/byline";
import { DocPage, type DocSearch } from "@/components/viewer/doc-page";
import { usePane } from "@/lib/pane";
import { useHome } from "@/lib/use-home";
import { cn } from "@/lib/utils";
import { useProjectMap } from "./project-data";

// The docs of a scope (the project, or a part with its features): which docs
// those are, the pillars they are grouped by (decision U8: Spec, UI,
// Architecture, Database, Research), and the Docs section: the list with a
// find field on the left, the open doc filling the window beside it (page text
// in a box with the comments at its right edge, or a design's canvas). On the
// phone the list is the page, and a doc opens full screen with a way back to
// the list.

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

/** The key under which the reader's choice to fold the doc list away beside a design is kept. */
const DOC_LIST_PANE = "omni:pane:doc-list";

/** Whether the doc list shows beside an open design, and the reader's way to fold it away or back. */
export type DocListToggle = { shown: boolean; onShown: (shown: boolean) => void };

/** The doc list's toggle beside a design: shown until the reader folds it away, the choice kept. */
export function useDocListToggle(): DocListToggle {
  const [shown, onShown] = usePane(DOC_LIST_PANE);
  return { shown, onShown };
}

/** The button in a design's header that folds the doc list away and back. */
function DocListButton({ list }: { list: DocListToggle }) {
  const Icon = list.shown ? PanelLeftClose : PanelLeftOpen;
  return (
    <button
      type="button"
      aria-label={list.shown ? "Hide the doc list" : "Show the doc list"}
      title={list.shown ? "Hide the doc list" : "Show the doc list"}
      onClick={() => list.onShown(!list.shown)}
      className="mr-2 -ml-1 grid size-(--control-h) shrink-0 place-items-center rounded-control text-ink-2 transition-colors duration-(--t-fast) hover:bg-hover hover:text-ink focus-visible:shadow-focus focus-visible:outline-none"
    >
      <Icon aria-hidden strokeWidth={2.25} className="size-(--icon)" />
    </button>
  );
}

/** Where a doc of the list opens: a router link's `to` and `params`. */
export type DocTarget = (doc: HomeDoc) => { to: string; params: Record<string, string> };

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
 * A Docs tab: the list on the left and on the right the open doc (`children`)
 * or, with no doc open, `empty`. With a doc open (`canvas`) the tab fills the
 * window under the bar, the list scrolling on its own, so an HTML doc and a
 * design sit in the same place at the same width. With none, the list sits in
 * the centred page (above `empty` in a window under 1280px).
 */
export function DocsTab({
  docs,
  current,
  target,
  about,
  failed,
  canvas = false,
  listShown = true,
  only,
  children,
}: {
  docs: readonly HomeDoc[] | null;
  current?: string;
  target: DocTarget;
  /** A quiet note after a doc's title (the feature a part's doc is about). */
  about?: (doc: HomeDoc) => string | undefined;
  failed?: boolean;
  /** A doc is open: the tab fills the window, the list on the left and the doc taking the rest. */
  canvas?: boolean;
  /** Beside an open doc, whether the list shows; the reader folds it away with the button in the doc's header. */
  listShown?: boolean;
  /** On the phone: the list alone, or the open doc alone (`children`, with its way back). */
  only?: "list" | "doc";
  children: ReactNode;
}) {
  const list = (
    <>
      {failed && <p role="alert" className="mb-2 text-danger-ink">The docs could not be loaded. Try again in a moment.</p>}
      {docs === null ? (
        <span aria-hidden className="block h-(--control-h) animate-pulse rounded-control bg-hover" />
      ) : (
        <DocList docs={docs} current={current} target={target} about={about} />
      )}
    </>
  );
  if (only === "list") {
    return (
      <div className="px-4 pt-5 pb-12">
        {docs?.length === 0 ? children : list}
      </div>
    );
  }
  if (only === "doc") {
    return <div className={cn("flex min-w-0 flex-col px-4 pt-3 pb-12", canvas && "h-full min-h-[560px]")}>{children}</div>;
  }
  if (canvas) {
    return (
      <div className="flex h-full min-h-[560px] flex-col md:flex-row">
        {listShown && (
          <aside aria-label="Doc list" className="shrink-0 border-line px-4 py-5 md:w-[calc(var(--doclist-w)+32px)] md:overflow-y-auto md:border-r">
            {list}
          </aside>
        )}
        <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto p-4 md:pl-6">{children}</div>
      </div>
    );
  }
  return (
    <div className="mx-auto grid w-full max-w-(--page-max) grid-cols-1 items-start gap-8 px-4 pt-8 pb-12 md:px-(--page-pad) xl:grid-cols-[var(--doclist-w)_minmax(0,1fr)] xl:gap-12">
      <aside aria-label="Doc list" className="max-xl:max-w-(--main-max) xl:sticky xl:top-8">
        {list}
      </aside>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

/** The open doc of a Docs tab: its title as the heading, then the doc under its slim bar. */
export function OpenDoc({
  workspaceId,
  projectId,
  docId,
  doc,
  search,
  heading = "h1",
  list,
}: {
  workspaceId: string;
  projectId: string;
  docId: string;
  /** The doc as the list has it; `undefined` while the list loads. */
  doc: HomeDoc | undefined;
  search: DocSearch;
  /** The heading's level: h1 on a Docs page, h2 under an object's own title (a feature's Docs tab). */
  heading?: "h1" | "h2";
  /** Given (beside the list, not on the phone), the doc fills the room it is given and its header has the button that folds the list away and back. */
  list?: DocListToggle;
}) {
  const Heading = heading;
  const fill = !!list && doc?.kind !== "design-system";
  return (
    <div className={cn("flex min-w-0 flex-col", fill && "min-h-0 flex-1")}>
      <div className="flex min-h-(--control-h-lg) items-center">
        {list && <DocListButton list={list} />}
        <Heading className="min-w-0 text-title font-semibold">
          {doc ? doc.title : <span aria-hidden className="block h-5 w-48 animate-pulse rounded-control bg-hover" />}
        </Heading>
      </div>
      <DocPage key={docId} workspaceId={workspaceId} projectId={projectId} docId={docId} search={search} chrome={{ meta: doc && <DocByline doc={doc} />, fill }} />
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
  const open = docId ? docs?.find((doc) => doc.id === docId) : undefined;
  const list = useDocListToggle();
  const phone = useIsPhone();
  // On the phone a design alone fills the screen; an HTML doc is as tall as it is and the page scrolls.
  const canvas = phone ? open?.kind === "design" : !!docId;
  const index = partId ? partLink(workspaceId, projectId, partId, "docs") : projectLink(workspaceId, projectId, "docs");
  return (
    <DocsTab
      docs={docs}
      current={docId}
      target={target}
      about={about}
      failed={failed}
      canvas={canvas}
      listShown={!canvas || list.shown}
      only={phone ? (docId ? "doc" : "list") : undefined}
    >
      {docId ? (
        <>
          {phone && (
            <Link
              {...index}
              activeOptions={{ exact: true }}
              activeProps={{}}
              className="-ml-1.5 inline-flex h-(--control-h) w-fit items-center gap-1 rounded-control pr-2 pl-0.5 font-medium text-ink-2 transition-colors duration-(--t-fast) hover:bg-hover hover:text-ink focus-visible:shadow-focus focus-visible:outline-none"
            >
              <ChevronLeft aria-hidden className="size-(--icon)" strokeWidth={2.25} />
              All docs
            </Link>
          )}
          <OpenDoc workspaceId={workspaceId} projectId={projectId} docId={docId} doc={open} search={search ?? {}} list={phone ? undefined : list} />
        </>
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
