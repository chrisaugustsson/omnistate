import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowLeft, FileText, PenTool } from "lucide-react";
import {
  loadActivity,
  loadActivityEntry,
  loadBoardAt,
  loadComponentAt,
  type ActivityDetail,
  type ActivityItem,
} from "@/api/activity";
import { loadDoc, loadHeadHtml } from "@/api/docs";
import type { ActivityAgent, ActivityEntry, DocBody } from "@/api/generated";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { ViewerToolbar } from "@/components/viewer/chrome";
import { designSnapshotName } from "@/components/viewer/comments";
import { drawComponent, drawSnapshot } from "@/design/thumbnail";
import { timeAgo } from "@/lib/format";
import { useReloading } from "@/lib/reloading";
import { useVisit } from "@/lib/visits";
import { cn } from "@/lib/utils";
import { ViewerFrame } from "@/viewer/frame";

// The Activity tab of a doc: the list of writes to it, newest first, and for
// one write what it changed, the state before and after side by side. An HTML
// doc's elements are shown in the sandboxed frame of the doc viewer, a
// design's boards and a design system's components are drawn by the renderer
// from the state they had at the write, and a rename and the tokens are text.
// A pair is drawn only once it scrolls into view, because a write may touch
// hundreds of boards.

type Params = { workspaceId: string; projectId: string; docId: string };

const NO_PINS: { node: string; count: number }[] = [];
/** The size of a board's or component's picture, in CSS pixels. */
const PICTURE = { width: 360, height: 220 };

function agentLabel(agent: ActivityAgent): string {
  const role = agent.role || "agent";
  return agent.model ? `${role} · ${agent.model}` : role;
}

/** The doc, for the toolbar's breadcrumb, with the Activity tab open. */
function DocShell({ workspaceId, projectId, docId, children }: Params & { children: (doc: DocBody) => ReactNode }) {
  const doc = useReloading(useCallback(() => loadDoc(workspaceId, projectId, docId), [workspaceId, projectId, docId]));
  const loaded = doc.value?.id === docId ? doc.value : null;
  useVisit(projectId, loaded ? (loaded.feature_id ?? "app") : undefined, loaded ? docId : undefined);
  return (
    <main className="flex min-h-full flex-col">
      <ViewerToolbar
        workspaceId={workspaceId}
        projectId={projectId}
        doc={doc.value}
        icon={doc.value?.kind === "html" ? <FileText /> : <PenTool />}
        docId={docId}
        view="activity"
      />
      <div className="mx-auto w-full max-w-6xl flex-1 px-5 py-8 md:px-10">
        {doc.failed && (
          <Alert variant="destructive">
            <AlertDescription>The doc could not be loaded. Try again in a moment.</AlertDescription>
          </Alert>
        )}
        {doc.value && children(doc.value)}
      </div>
    </main>
  );
}

// The list -----------------------------------------------------------------------------

/** The Activity tab: every write to the doc, newest first, a page at a time. */
export function ActivityPage({ workspaceId, projectId, docId }: Params) {
  return (
    <DocShell workspaceId={workspaceId} projectId={projectId} docId={docId}>
      {() => <ActivityList workspaceId={workspaceId} projectId={projectId} docId={docId} />}
    </DocShell>
  );
}

function ActivityList({ workspaceId, projectId, docId }: Params) {
  const [entries, setEntries] = useState<ActivityEntry[] | null>(null);
  const [next, setNext] = useState<number | null>(null);
  const [failed, setFailed] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setEntries(null);
    setFailed(false);
    loadActivity(workspaceId, projectId, docId).then(
      (page) => {
        if (cancelled) return;
        setEntries(page.entries);
        setNext(page.next_before ?? null);
      },
      () => {
        if (!cancelled) setFailed(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [workspaceId, projectId, docId]);

  async function more() {
    if (next === null) return;
    setLoadingMore(true);
    setFailed(false);
    try {
      const page = await loadActivity(workspaceId, projectId, docId, next);
      setEntries((now) => [...(now ?? []), ...page.entries]);
      setNext(page.next_before ?? null);
    } catch {
      setFailed(true);
    } finally {
      setLoadingMore(false);
    }
  }

  return (
    <section aria-labelledby="activity-title" className="space-y-4">
      <h2 id="activity-title" className="font-display text-[1.5rem] leading-tight tracking-[-0.01em]">
        Activity
      </h2>
      {failed && (
        <Alert variant="destructive">
          <AlertDescription>The activity could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {entries === null && !failed && <div className="h-14 animate-pulse rounded-xl bg-muted" aria-hidden />}
      {entries?.length === 0 && (
        <p className="rounded-xl border border-dashed px-4 py-8 text-center text-sm text-muted-foreground">
          Nothing has been written to this doc yet.
        </p>
      )}
      {entries && entries.length > 0 && (
        <ul className="divide-y overflow-hidden rounded-xl border bg-card">
          {entries.map((entry) => (
            <li key={entry.seq}>
              <Link
                to="/w/$workspaceId/projects/$projectId/docs/$docId/activity/$seq"
                params={{ workspaceId, projectId, docId, seq: String(entry.seq) }}
                className="flex items-center gap-3 px-4 py-3 text-sm transition-colors hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none"
              >
                <span className="min-w-0 flex-1 truncate font-medium">{entry.summary}</span>
                <span className="hidden w-48 shrink-0 truncate font-mono text-xs text-muted-foreground sm:inline">
                  {agentLabel(entry.agent)}
                </span>
                <time
                  dateTime={entry.created_at}
                  title={new Date(entry.created_at).toLocaleString()}
                  className="w-24 shrink-0 text-right text-xs text-muted-foreground"
                >
                  {timeAgo(entry.created_at)}
                </time>
              </Link>
            </li>
          ))}
        </ul>
      )}
      {next !== null && (
        <Button type="button" variant="outline" size="sm" disabled={loadingMore} onClick={() => void more()}>
          Load more
        </Button>
      )}
    </section>
  );
}

// One write ----------------------------------------------------------------------------

/** One write and what it changed, before and after. */
export function ActivityEntryPage({ workspaceId, projectId, docId, seq }: Params & { seq: number }) {
  return (
    <DocShell workspaceId={workspaceId} projectId={projectId} docId={docId}>
      {(doc) => <EntryView workspaceId={workspaceId} projectId={projectId} docId={docId} seq={seq} doc={doc} />}
    </DocShell>
  );
}

function EntryView({ workspaceId, projectId, docId, seq, doc }: Params & { seq: number; doc: DocBody }) {
  const entry = useReloading(
    useCallback(() => loadActivityEntry(workspaceId, projectId, docId, seq), [workspaceId, projectId, docId, seq]),
  );
  const detail = entry.value;
  return (
    <div className="space-y-8">
      <Link
        to="/w/$workspaceId/projects/$projectId/docs/$docId/activity"
        params={{ workspaceId, projectId, docId }}
        className="group inline-flex items-center gap-1.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
      >
        <ArrowLeft className="size-3.5 transition-transform group-hover:-translate-x-0.5" />
        Activity
      </Link>
      {entry.failed && (
        <Alert variant="destructive">
          <AlertDescription>The change could not be loaded. It may not exist.</AlertDescription>
        </Alert>
      )}
      {detail && (
        <>
          <header className="space-y-1">
            <h2 className="font-display text-[1.5rem] leading-tight tracking-[-0.01em]">{detail.summary}</h2>
            <p className="text-xs text-muted-foreground">
              <span className="font-mono">{agentLabel(detail.agent)}</span>
              {" · "}
              <time dateTime={detail.created_at} title={new Date(detail.created_at).toLocaleString()}>
                {timeAgo(detail.created_at)}
              </time>
            </p>
          </header>
          <Changes workspaceId={workspaceId} projectId={projectId} docId={docId} doc={doc} detail={detail} />
        </>
      )}
    </div>
  );
}

function Changes({
  workspaceId,
  projectId,
  docId,
  doc,
  detail,
}: Params & { doc: DocBody; detail: ActivityDetail }) {
  const styles = useDocStyles(workspaceId, projectId, docId, doc.kind === "html" && detail.items.length > 0);
  const system = useDesignSystem(workspaceId, projectId, docId, doc.kind === "design-system" && detail.items.length > 0);
  return (
    <div className="space-y-8">
      {detail.title && (
        <Pair label="Title" before={<Text>{detail.title.before}</Text>} after={<Text>{detail.title.after}</Text>} />
      )}
      {detail.items.map((item, index) => (
        <ItemPair
          key={index}
          item={item}
          styles={styles}
          system={system}
          workspaceId={workspaceId}
          projectId={projectId}
          docId={docId}
        />
      ))}
      {detail.tokens && (
        <Pair
          label="Tokens"
          before={detail.tokens.before ? <Json>{detail.tokens.before}</Json> : null}
          after={detail.tokens.after ? <Json>{detail.tokens.after}</Json> : null}
        />
      )}
    </div>
  );
}

/** The doc's own `<style>` elements, as the head has them now: what the frames of its elements add to the default stylesheet. */
function useDocStyles(workspaceId: string, projectId: string, docId: string, wanted: boolean): string {
  const [styles, setStyles] = useState("");
  useEffect(() => {
    if (!wanted) return;
    let cancelled = false;
    loadHeadHtml(workspaceId, projectId, docId).then(
      (html) => {
        if (cancelled) return;
        const parsed = new DOMParser().parseFromString(html, "text/html");
        setStyles(Array.from(parsed.querySelectorAll("style"), (style) => style.outerHTML).join("\n"));
      },
      () => undefined,
    );
    return () => {
      cancelled = true;
    };
  }, [workspaceId, projectId, docId, wanted]);
  return styles;
}

function systemOf(html: string): Record<string, unknown> {
  try {
    return JSON.parse(html) as Record<string, unknown>;
  } catch {
    return {};
  }
}

/** The design system as its head has it now, the tokens with the components under `$components`, which a component drawn at an earlier write is resolved against; `null` until read. */
function useDesignSystem(workspaceId: string, projectId: string, docId: string, wanted: boolean) {
  const [system, setSystem] = useState<Record<string, unknown> | null>(null);
  useEffect(() => {
    if (!wanted) return;
    let cancelled = false;
    loadHeadHtml(workspaceId, projectId, docId).then(
      (html) => {
        if (!cancelled) setSystem(systemOf(html));
      },
      () => {
        if (!cancelled) setSystem({});
      },
    );
    return () => {
      cancelled = true;
    };
  }, [workspaceId, projectId, docId, wanted]);
  return system;
}

function ItemPair({
  item,
  styles,
  system,
  workspaceId,
  projectId,
  docId,
}: Params & { item: ActivityItem; styles: string; system: ReturnType<typeof useDesignSystem> }) {
  if (item.kind === "node") {
    return (
      <Pair
        label={item.node ? `Element ${item.node}` : "Whole doc"}
        lazy
        before={item.before === null ? null : <Frame html={item.before} styles={styles} workspaceId={workspaceId} title="Before" />}
        after={item.after === null ? null : <Frame html={item.after} styles={styles} workspaceId={workspaceId} title="After" />}
      />
    );
  }
  if (item.kind === "board") {
    const at = (seq: number) => async (): Promise<Drawn> => {
      const tags = await loadBoardAt(workspaceId, projectId, docId, item.board, seq);
      return { name: designSnapshotName(tags), draw: (target) => drawSnapshot(tags, target, PICTURE) };
    };
    return (
      <Pair
        label={`Board ${item.board}`}
        lazy
        before={item.before_at === null ? null : <Picture load={at(item.before_at)} />}
        after={item.after_at === null ? null : <Picture load={at(item.after_at)} />}
      />
    );
  }
  const at = (seq: number) => async (): Promise<Drawn> => {
    const node = await loadComponentAt(workspaceId, projectId, item.component, seq);
    // The component as it was, among the components of the head, so that the ones it places resolve.
    const components = Array.isArray(system?.$components) ? (system.$components as { id?: unknown }[]) : [];
    const others = components.filter((other) => other.id !== node.id);
    return {
      name: typeof node.name === "string" ? node.name : "",
      draw: (target) => drawComponent(node, { ...system, $components: [...others, node] }, target, PICTURE),
    };
  };
  const waiting = <div aria-hidden className="h-24 animate-pulse rounded-lg bg-muted" />;
  const side = (seq: number | null) => (seq === null ? null : system ? <Picture load={at(seq)} /> : waiting);
  return <Pair label={`Component ${item.component}`} lazy before={side(item.before_at)} after={side(item.after_at)} />;
}

/** Two columns, Before and After; a side that is `null` says the thing was not there. */
function Pair({
  label,
  before,
  after,
  lazy,
}: {
  label: string;
  before: ReactNode;
  after: ReactNode;
  lazy?: boolean;
}) {
  const sides = (
    <div className="grid gap-4 md:grid-cols-2">
      <Side caption="Before">{before}</Side>
      <Side caption="After">{after}</Side>
    </div>
  );
  return (
    <section aria-label={label} className="space-y-2">
      <h3 className="font-mono text-xs text-muted-foreground">{label}</h3>
      {lazy ? <WhenVisible>{sides}</WhenVisible> : sides}
    </section>
  );
}

function Side({ caption, children }: { caption: string; children: ReactNode }) {
  return (
    <div className="min-w-0 space-y-1.5">
      <div className="text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">{caption}</div>
      {children ?? (
        <p className="rounded-lg border border-dashed px-3 py-6 text-center text-sm text-muted-foreground">Not present</p>
      )}
    </div>
  );
}

/** Renders `children` once the box has scrolled near the view; the box holds some room until then. */
function WhenVisible({ children }: { children: ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  const [seen, setSeen] = useState(typeof IntersectionObserver === "undefined");
  useEffect(() => {
    const element = box.current;
    if (seen || !element) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setSeen(true);
          observer.disconnect();
        }
      },
      { rootMargin: "300px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [seen]);
  return (
    <div ref={box} className={cn(!seen && "min-h-64")}>
      {seen ? children : null}
    </div>
  );
}

/** Text as it is, escaped. */
function Text({ children }: { children: string }) {
  return <p className="rounded-lg border bg-card px-3 py-2 text-sm break-words whitespace-pre-wrap">{children}</p>;
}

/** DTCG tokens as formatted JSON. */
function Json({ children }: { children: Record<string, unknown> }) {
  return (
    <pre className="max-h-96 overflow-auto rounded-lg border bg-card p-3 font-mono text-[12px] leading-relaxed">
      {JSON.stringify(children, null, 2)}
    </pre>
  );
}

/** An element of an HTML doc in a sandboxed frame, with the default stylesheet and the doc's own styles. */
function Frame({ html, styles, workspaceId, title }: { html: string; styles: string; workspaceId: string; title: string }) {
  const shown = useMemo(() => `<div>${styles}${html}</div>`, [styles, html]);
  return (
    <ViewerFrame
      workspaceId={workspaceId}
      html={shown}
      pins={NO_PINS}
      selected={null}
      onSelect={noop}
      className="h-48 w-full rounded-md border bg-white"
      title={title}
    />
  );
}

function noop() {}

/** What a picture shows once it is read: the name of the node and how to draw it. */
type Drawn = { name: string; draw: (target: HTMLCanvasElement) => Promise<void> };

/** A board or component, read and drawn by the renderer. */
function Picture({ load }: { load: () => Promise<Drawn> }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [state, setState] = useState<{ name: string } | "failed" | null>(null);
  useEffect(() => {
    let cancelled = false;
    load()
      .then(async (drawn) => {
        const target = canvas.current;
        if (!target) return;
        await drawn.draw(target);
        if (!cancelled) setState({ name: drawn.name });
      })
      .catch(() => {
        if (!cancelled) setState("failed");
      });
    return () => {
      cancelled = true;
    };
    // A picture is of one point in the doc's history, which never changes.
  }, []);
  return (
    <div className="space-y-1.5">
      {state === "failed" && (
        <p className="rounded-lg border border-dashed px-3 py-6 text-center text-sm text-muted-foreground">
          This could not be drawn
        </p>
      )}
      <canvas
        ref={canvas}
        role="img"
        aria-label={typeof state === "object" && state?.name ? `Picture of ${state.name}` : "Picture"}
        style={{ width: PICTURE.width, height: PICTURE.height }}
        className={cn("max-w-full rounded-md border bg-muted", state === "failed" && "hidden")}
      />
      {typeof state === "object" && state?.name && <p className="truncate text-xs font-medium">{state.name}</p>}
    </div>
  );
}
