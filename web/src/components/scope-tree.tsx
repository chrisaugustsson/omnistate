import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Boxes, ChevronRight, Database, FileText, FlaskConical, MessagesSquare, Palette, SquareKanban, type LucideIcon } from "lucide-react";
import type { HomeResponse } from "@/api/generated";
import { PILLARS, featureNames } from "@/components/project/items";
import { inOrder } from "@/components/project/scope-page";
import { cn } from "@/lib/utils";

// The part of the sidebar that shows one scope: its name, its five pillars
// with their docs, and its threads.

const PILLAR_ICONS: Record<string, LucideIcon> = {
  spec: FileText,
  ui: Palette,
  architecture: Boxes,
  db: Database,
  research: FlaskConical,
};

const rowClass =
  "flex items-center gap-2 rounded-md px-2 text-sidebar-foreground transition-colors hover:bg-sidebar-accent hover:text-foreground";
const activeClass = "bg-sidebar-accent text-foreground";

/** Whether the fold under `key` is open: open until the reader folds it, and kept in localStorage. */
export function useFold(key: string): [boolean, () => void] {
  const [, render] = useState(0);
  let stored: string | null = null;
  try {
    stored = localStorage.getItem(key);
  } catch {
    // Storage that is blocked only costs the folded state.
  }
  function toggle() {
    try {
      localStorage.setItem(key, stored === "closed" ? "open" : "closed");
    } catch {
      return;
    }
    render((count) => count + 1);
  }
  return [stored !== "closed", toggle];
}

function Pillar({
  workspaceId,
  projectId,
  scope,
  pillar,
  title,
  docs,
}: {
  workspaceId: string;
  projectId: string;
  scope: string;
  pillar: string;
  title: string;
  docs: HomeResponse["docs"];
}) {
  const [open, toggle] = useFold(`omni:fold:${projectId}:${scope}:${pillar}`);
  const Icon = PILLAR_ICONS[pillar] ?? FileText;
  if (docs.length === 0) {
    return (
      <div className={cn(rowClass, "h-7 text-xs text-muted-foreground hover:bg-transparent hover:text-muted-foreground")}>
        <ChevronRight aria-hidden className="size-3 shrink-0 opacity-0" />
        <Icon aria-hidden className="size-3.5 shrink-0" />
        <span>{title}</span>
        <span className="ml-auto text-[11px] italic">empty</span>
      </div>
    );
  }
  return (
    <div>
      <button type="button" aria-expanded={open} onClick={toggle} className={cn(rowClass, "h-7 w-full text-left text-xs font-medium")}>
        <ChevronRight aria-hidden className={cn("size-3 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")} />
        <Icon aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
        <span>{title}</span>
        <span className="ml-auto text-[11px] text-muted-foreground tabular-nums">{docs.length}</span>
      </button>
      {open && (
        <div className="ml-4 border-l pl-1">
          {inOrder(docs).map((doc) => (
            <Link
              key={doc.id}
              to="/w/$workspaceId/projects/$projectId/docs/$docId"
              params={{ workspaceId, projectId, docId: doc.id }}
              activeOptions={{ exact: false, includeSearch: false }}
              activeProps={{ className: activeClass }}
              className={cn(rowClass, "h-7 text-xs")}
            >
              <span className="min-w-0 flex-1 truncate">{doc.title}</span>
              {(doc.waiting ?? 0) > 0 && (
                <span
                  title="Waiting on you"
                  className="min-w-5 rounded-full bg-signal px-1.5 text-center text-[11px] leading-5 text-signal-foreground tabular-nums"
                >
                  {doc.waiting}
                </span>
              )}
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * One scope's tree: `scope` is "app" or a feature's ID. It starts with the
 * scope's name, linking to its page, then the five pillars with their docs,
 * and last the scope's threads.
 */
export function ScopeTree({
  workspaceId,
  projectId,
  scope,
  home,
}: {
  workspaceId: string;
  projectId: string;
  scope: string;
  home: HomeResponse;
}) {
  const params = { workspaceId, projectId };
  const own = (item: { feature_id?: string | null }) => (item.feature_id ?? "app") === scope;
  const docs = home.docs.filter(own);
  const threads = home.threads.filter(own).length;
  const openStories = home.features?.find((summary) => summary.feature.id === scope)?.stories?.open ?? 0;
  const name = scope === "app" ? "App" : (featureNames(home).get(scope) ?? "");
  const headClass = cn(rowClass, "h-[30px] text-[13px] font-semibold text-foreground");
  return (
    <div className="space-y-0.5">
      {scope === "app" ? (
        <Link
          to="/w/$workspaceId/projects/$projectId/app"
          params={params}
          activeOptions={{ exact: true, includeSearch: false }}
          activeProps={{ className: activeClass }}
          className={headClass}
        >
          <span className="truncate">{name}</span>
        </Link>
      ) : (
        <Link
          to="/w/$workspaceId/projects/$projectId/features/$featureId"
          params={{ ...params, featureId: scope }}
          activeOptions={{ exact: true, includeSearch: false }}
          activeProps={{ className: activeClass }}
          className={headClass}
        >
          <span className="truncate">{name}</span>
          <span className="ml-auto text-[11px] font-medium text-muted-foreground">Feature</span>
        </Link>
      )}
      {PILLARS.map(({ pillar, title }) => (
        <Pillar
          key={pillar}
          workspaceId={workspaceId}
          projectId={projectId}
          scope={scope}
          pillar={pillar}
          title={title}
          docs={docs.filter((doc) => doc.pillar === pillar)}
        />
      ))}
      <Link
        to="/w/$workspaceId/projects/$projectId/threads"
        params={params}
        search={{ scope }}
        activeOptions={{ exact: true, includeSearch: true }}
        activeProps={{ className: activeClass }}
        className={cn(rowClass, "mt-0.5 h-7 text-xs font-medium")}
      >
        <MessagesSquare aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
        Threads ({threads})
      </Link>
      {scope !== "app" && (
        <Link
          to="/w/$workspaceId/projects/$projectId/features/$featureId/board"
          params={{ ...params, featureId: scope }}
          activeOptions={{ exact: true, includeSearch: false }}
          activeProps={{ className: activeClass }}
          className={cn(rowClass, "h-7 text-xs font-medium")}
        >
          <SquareKanban aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
          Board ({openStories})
        </Link>
      )}
    </div>
  );
}
