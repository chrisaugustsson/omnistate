import { useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { Check, Plus, Search } from "lucide-react";
import type { PartBody, ProjectBody } from "@/api/generated";
import { partTones } from "@/lib/identity";
import { cn } from "@/lib/utils";
import { useWorkspaceParts } from "@/lib/use-workspace";
import { PartIcon, ProjectMark } from "./part-icon";
import { TAB_LABELS, partLink, partTab, projectLink, projectTab, type Tab } from "./scope";

// The scope switcher: a popover under the scope button. A find field; the
// workspace's projects on the left; on the right the scopes of the project in
// view (the current one, or the project row you point at): Project-wide, then
// its parts with their kind. Every row opens the same tab of the scope you pick
// ("You stay on Tracks"). ↑↓ move in a column, ←→ between the columns, ↵ opens,
// Esc closes.

const option =
  "flex h-8 w-full items-center gap-2.5 rounded-control px-2 font-medium whitespace-nowrap text-ink outline-none transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover focus-visible:bg-hover aria-[current=page]:bg-selected aria-[current=true]:bg-selected";

function Option({ label, kind, end, check }: { label: string; kind?: string; end?: ReactNode; check?: boolean }) {
  return (
    <>
      <span className="min-w-0 truncate">{label}</span>
      {kind && <span className="text-meta font-normal text-ink-3">{kind}</span>}
      {(end || check) && (
        <span className="ml-auto flex items-center gap-2 text-meta font-normal text-ink-3 tabular-nums">
          {end}
          {check && <Check aria-hidden className="size-(--icon-sm) text-ink" strokeWidth={2.25} />}
        </span>
      )}
    </>
  );
}

const Separator = () => <div aria-hidden className="mx-2 my-1 h-px bg-line" />;

/** The parts of a project with their tones, oldest first, for its mark. */
function markParts(parts: readonly PartBody[] | undefined) {
  if (!parts) return undefined;
  const tones = partTones(parts);
  return [...parts]
    .sort((a, b) => (tones.get(a.id) ?? 0) - (tones.get(b.id) ?? 0))
    .map((part) => ({ kind: part.kind, tone: tones.get(part.id) ?? 1 }));
}

const matching = (query: string, ...texts: (string | undefined)[]) =>
  texts.some((text) => text?.toLowerCase().includes(query));

export function Switcher({
  workspaceId,
  workspaceName,
  projects,
  projectId,
  partId,
  tab,
  home,
  onClose,
}: {
  workspaceId: string;
  workspaceName: string;
  projects: readonly ProjectBody[];
  /** The current project; on Home, none. */
  projectId?: string;
  /** The current part; none on a project's own pages. */
  partId?: string;
  /** The lit tab, kept when switching. */
  tab: Tab | undefined;
  /** Whether the bar is on Home. */
  home: boolean;
  onClose: () => void;
}) {
  const { parts: allParts } = useWorkspaceParts(workspaceId, projects);
  const [query, setQuery] = useState("");
  const [pointed, setPointed] = useState<string | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const wanted = query.trim().toLowerCase();

  const viewedId = pointed ?? projectId ?? projects[0]?.id;
  const viewed = projects.find((project) => project.id === viewedId);
  const viewedParts = useMemo(() => {
    const parts = (viewedId && allParts?.get(viewedId)) || [];
    const tones = partTones(parts);
    return [...parts]
      .sort((a, b) => (tones.get(a.id) ?? 0) - (tones.get(b.id) ?? 0))
      .map((part) => ({ part, tone: tones.get(part.id) ?? 1 }));
  }, [allParts, viewedId]);

  const shownProjects = projects.filter((project) => !wanted || matching(wanted, project.name));
  const projectWide = !wanted || matching(wanted, "Project-wide", "all parts");
  const shownParts = viewedParts.filter(({ part }) => !wanted || matching(wanted, part.name, part.kind));
  const onProject = !home && !partId && viewedId === projectId;

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const box = root.current;
    if (!box) return;
    const target = event.target as HTMLElement;
    const columns = Array.from(box.querySelectorAll<HTMLElement>("[data-sw-col]"));
    const scopes = columns[columns.length - 1];
    const options = (column: Element) => Array.from(column.querySelectorAll<HTMLElement>("[data-sw-opt]"));
    const currentIn = (column: Element) => column.querySelector<HTMLElement>("[data-sw-opt][aria-current]") ?? options(column)[0];
    const inInput = target.matches("input");
    if (event.key === "Enter" && inInput) {
      const first = options(scopes)[0] ?? options(columns[0])[0];
      if (first) {
        event.preventDefault();
        first.click();
      }
      return;
    }
    if (!["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight"].includes(event.key)) return;
    if (inInput && (event.key === "ArrowLeft" || event.key === "ArrowRight")) return;
    event.preventDefault();
    const column = target.closest("[data-sw-col]");
    if (inInput || !column) {
      (currentIn(scopes) ?? currentIn(columns[0]))?.focus();
      return;
    }
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      const other = columns[(columns.indexOf(column as HTMLElement) + 1) % columns.length];
      currentIn(other)?.focus();
      return;
    }
    const list = options(column);
    const at = list.indexOf(target);
    if (event.key === "ArrowUp" && at <= 0) {
      box.querySelector<HTMLInputElement>("input")?.focus();
      return;
    }
    list[Math.min(list.length - 1, Math.max(0, at + (event.key === "ArrowDown" ? 1 : -1)))]?.focus();
  }

  return (
    <div
      ref={root}
      role="dialog"
      aria-label="Switch project or part"
      onKeyDown={onKeyDown}
      onClick={(event) => {
        if ((event.target as HTMLElement).closest("a")) onClose();
      }}
      className="absolute top-[calc(100%+6px)] left-0 z-40 w-(--switcher-w) max-w-[calc(100vw-24px)] origin-top-left animate-pop-in overflow-hidden rounded-sheet bg-sheet shadow-pop"
    >
      <label className="flex h-11 items-center gap-2.5 border-b border-line px-4">
        <Search aria-hidden className="size-(--icon-sm) shrink-0 text-faint" strokeWidth={2.25} />
        <input
          type="text"
          autoFocus
          aria-label="Find a project or part"
          placeholder="Find a project or part"
          autoComplete="off"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-ink-3"
        />
      </label>
      <div className="grid grid-cols-[248px_minmax(0,1fr)]">
        <div data-sw-col role="group" aria-label="Projects" className="min-w-0 p-2 pr-1">
          <div className="flex h-7 items-center px-2 text-meta font-medium text-ink-3">Projects in {workspaceName}</div>
          {shownProjects.map((project) => {
            const parts = allParts?.get(project.id);
            return (
              <Link
                key={project.id}
                data-sw-opt
                {...projectLink(workspaceId, project.id, projectTab(tab))}
                activeOptions={{ exact: true }}
                activeProps={{}}
                aria-current={project.id === projectId ? "true" : undefined}
                onPointerEnter={() => setPointed(project.id)}
                onFocus={() => setPointed(project.id)}
                className={option}
              >
                <ProjectMark name={project.name} parts={markParts(parts)} />
                <Option label={project.name} end={parts ? `${parts.length} ${parts.length === 1 ? "part" : "parts"}` : undefined} />
              </Link>
            );
          })}
          {wanted && shownProjects.length === 0 && <p className="px-2 py-1.5 text-meta text-ink-3">No project matches</p>}
        </div>
        <div data-sw-col role="group" aria-label={viewed?.name ?? "Scopes"} className="min-w-0 border-l border-line p-2 pl-1">
          <div className="flex h-7 items-center truncate px-2 text-meta font-medium text-ink-3">{viewed?.name}</div>
          {viewed && projectWide && (
            <Link
              data-sw-opt
              {...projectLink(workspaceId, viewed.id, projectTab(tab))}
              activeOptions={{ exact: true }}
              activeProps={{}}
              aria-current={onProject ? "true" : undefined}
              className={option}
            >
              <ProjectMark name={viewed.name} parts={markParts(allParts?.get(viewed.id))} />
              <Option label="Project-wide" kind="all parts" check={onProject} />
            </Link>
          )}
          {!wanted && shownParts.length > 0 && <Separator />}
          {viewed &&
            shownParts.map(({ part, tone }) => {
              const current = !home && viewed.id === projectId && part.id === partId;
              return (
                <Link
                  key={part.id}
                  data-sw-opt
                  {...partLink(workspaceId, viewed.id, part.id, partTab(tab))}
                  activeOptions={{ exact: true }}
                  activeProps={{}}
                  aria-current={current ? "true" : undefined}
                  className={option}
                >
                  <PartIcon name={part.name} kind={part.kind} tone={tone} />
                  <Option label={part.name} kind={part.kind} check={current} />
                </Link>
              );
            })}
          {wanted && !projectWide && shownParts.length === 0 && <p className="px-2 py-1.5 text-meta text-ink-3">No part matches</p>}
          {viewed && !wanted && (
            <>
              <Separator />
              <Link
                data-sw-opt
                {...projectLink(workspaceId, viewed.id, "parts")}
                activeOptions={{ exact: true }}
                activeProps={{}}
                className={cn(option, "text-ink-2")}
              >
                <Plus aria-hidden className="mx-px size-(--icon-sm) text-faint" strokeWidth={2.25} />
                New part
              </Link>
            </>
          )}
        </div>
      </div>
      <div className="flex h-9 items-center gap-4 border-t border-line bg-frame px-4 text-meta text-ink-3">
        <span>
          {home ? "Parts open on " : "You stay on "}
          <b className="font-medium text-ink-2">{home || !tab ? "Overview" : TAB_LABELS[tab]}</b>
        </span>
        <span aria-hidden className="ml-auto hidden items-center gap-3 sm:flex">
          <Keys keys={["↑", "↓"]}>move</Keys>
          <Keys keys={["←", "→"]}>column</Keys>
          <Keys keys={["↵"]}>open</Keys>
          <Keys keys={["esc"]}>close</Keys>
        </span>
      </div>
    </div>
  );
}

/** A keyboard key as the bar draws it: 18px, 11/500, on the hover fill. */
export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <kbd className={cn("inline-flex h-[18px] items-center rounded-badge bg-hover px-[5px] font-sans text-[11px] leading-none font-medium text-ink-3 tabular-nums", className)}>
      {children}
    </kbd>
  );
}

function Keys({ keys, children }: { keys: string[]; children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1">
      {keys.map((key) => (
        <Kbd key={key}>{key}</Kbd>
      ))}
      {children}
    </span>
  );
}
