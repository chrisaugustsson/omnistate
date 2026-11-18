import { useCallback, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { Box, Check, ChevronDown, Plus, Search } from "lucide-react";
import type { FeatureBody, HomeResponse, PartBody, ProjectBody, TrackSummary } from "@/api/generated";
import { loadFeatures } from "@/api/features";
import { loadParts } from "@/api/parts";
import { partTones } from "@/lib/identity";
import { useReloading } from "@/lib/reloading";
import { useWorkspaceParts } from "@/lib/use-workspace";
import { useProjectsEvents } from "@/live/context";
import { cn } from "@/lib/utils";
import { PartIcon, ProjectMark } from "./part-icon";
import { usePopover } from "./popover";
import {
  SECTION_LABELS,
  featureLink,
  featureSection,
  partLink,
  partSection,
  projectLink,
  projectSection,
  trackLink,
  trackSection,
  workspaceLink,
  type Level,
  type Scope,
  type Section,
  type SectionLink,
} from "./scope";
import { BottomSheet } from "./sheet";
import { StateIcon, inFlight } from "./state-icon";

// The path (navigation F): workspace ⌄ / project ⌄ / part ⌄ / feature ⌄ or
// track ⌄. Each step's name links to that level's first section; its ⌄ opens a
// menu of its siblings (a popover with a find field on a desktop, a bottom
// sheet with the path above it on the phone). Choosing a sibling keeps the
// section you are on where the sibling has it ("You stay on Tracks").

export type Workspace = { id: string; name: string };

/** What the path knows: the open level's chain of objects, as the shell loads them. */
export type PathData = {
  scope: Scope;
  workspace: Workspace;
  workspaces: readonly Workspace[];
  projects: readonly ProjectBody[] | null;
  project: ProjectBody | undefined;
  /** The open project's parts; null while they load. */
  parts: readonly PartBody[] | null;
  /** The part on the path: the URL's, or a track's own. */
  partId: string | undefined;
  part: PartBody | undefined;
  /** The open project's home (its tracks, docs, what waits). */
  home: HomeResponse | undefined;
  /** The open part's features, on a feature's pages. */
  features: readonly FeatureBody[] | null;
  feature: FeatureBody | undefined;
  track: TrackSummary | undefined;
  /** Opens a create dialog the header holds (a part, a track). */
  onCreate: (what: "part" | "track") => void;
};

/** One step of the path. */
export type Crumb = { level: Level; name: string; mark?: ReactNode; link: SectionLink };

const NOUN: Record<Level, string> = { workspace: "workspace", project: "project", part: "part", feature: "feature", track: "track" };

/** A project's parts with their tones, oldest first, for its mark. */
export function markParts(parts: readonly PartBody[] | null | undefined) {
  if (!parts) return undefined;
  const tones = partTones(parts);
  return [...parts]
    .sort((a, b) => (tones.get(a.id) ?? 0) - (tones.get(b.id) ?? 0))
    .map((part) => ({ kind: part.kind, tone: tones.get(part.id) ?? 1 }));
}

/** The open project's parts, or null off a project; loaded again on the project's live events. */
export function useOpenParts(workspaceId: string, projectId: string | undefined) {
  const load = useCallback(() => (projectId ? loadParts(workspaceId, projectId) : Promise.resolve(null)), [workspaceId, projectId]);
  const { value, reload } = useReloading(load);
  useProjectsEvents(projectId ? [projectId] : [], reload);
  return value;
}

/** A part's features, or null when `load` is off; loaded again on the project's live events. */
export function usePartFeatures(workspaceId: string, projectId: string | undefined, partId: string | undefined, on: boolean) {
  const load = useCallback(
    () => (on && projectId && partId ? loadFeatures(workspaceId, projectId, partId) : Promise.resolve(null)),
    [workspaceId, projectId, partId, on],
  );
  const { value, reload } = useReloading(load);
  useProjectsEvents(on && projectId ? [projectId] : [], reload);
  return value;
}

/** The section a crumb's siblings open on: the one you are on, or, below the crumb, the section that lists what you are in. */
function keptSection(crumb: Level, scope: Scope): Section | undefined {
  if (crumb === scope.level) return scope.section;
  if (scope.level === "track") return "tracks";
  if (scope.level === "feature") return crumb === "part" ? "features" : "parts";
  return scope.section;
}

/** The path's steps, workspace first. */
export function useCrumbs(data: PathData): Crumb[] {
  const { scope, workspace, project, parts, partId, part, feature, track } = data;
  return useMemo(() => {
    const ws = workspace.id;
    const crumbs: Crumb[] = [{ level: "workspace", name: workspace.name, link: workspaceLink(ws, "inbox") }];
    if (scope.level === "workspace" || !scope.projectId) return crumbs;
    const projectId = scope.projectId;
    crumbs.push({
      level: "project",
      name: project?.name ?? "",
      mark: <ProjectMark name={project?.name ?? ""} parts={markParts(parts)} />,
      link: projectLink(ws, projectId, "overview"),
    });
    if (scope.level === "project") return crumbs;
    if (partId) {
      const tone = parts ? (partTones(parts).get(partId) ?? 1) : 1;
      crumbs.push({
        level: "part",
        name: part?.name ?? "",
        mark: part ? <PartIcon name={part.name} kind={part.kind} tone={tone} /> : <span aria-hidden className="size-(--icon) shrink-0 rounded-badge bg-hover" />,
        link: partLink(ws, projectId, partId, "overview"),
      });
    }
    if (scope.level === "feature" && partId && scope.featureId) {
      crumbs.push({
        level: "feature",
        name: feature?.name ?? "",
        mark: <Box aria-hidden strokeWidth={2.25} className="size-(--icon-sm) shrink-0 text-ink-3" />,
        link: featureLink(ws, projectId, partId, scope.featureId, "overview"),
      });
    }
    if (scope.level === "track" && scope.trackId) {
      crumbs.push({
        level: "track",
        name: track?.track.name ?? "",
        mark: track ? <StateIcon state={track.track.state} /> : undefined,
        link: trackLink(ws, projectId, scope.trackId, "spec"),
      });
    }
    return crumbs;
  }, [scope, workspace, project, parts, partId, part, feature, track]);
}

/** Where the phone's ‹ leads from the open level: up to the list that holds it. */
export function upLink(data: PathData): { link: SectionLink; name: string } | undefined {
  const { scope, workspace, project, partId, part } = data;
  const ws = workspace.id;
  const projectId = scope.projectId ?? "";
  switch (scope.level) {
    case "workspace":
      return undefined;
    case "project":
      return { link: workspaceLink(ws, "projects"), name: workspace.name };
    case "part":
      return { link: projectLink(ws, projectId, "parts"), name: project?.name ?? "" };
    case "feature":
      return partId ? { link: partLink(ws, projectId, partId, "features"), name: part?.name ?? "" } : undefined;
    case "track":
      return partId ? { link: partLink(ws, projectId, partId, "tracks"), name: part?.name ?? "" } : { link: projectLink(ws, projectId, "tracks"), name: project?.name ?? "" };
  }
}

// ── Menus ────────────────────────────────────────────────────────────────

type Option = {
  key: string;
  label: string;
  /** A muted word after the label (a part's kind). */
  kind?: string;
  mark?: ReactNode;
  /** Muted, at the end of the row ("2 parts"). */
  end?: ReactNode;
  current?: boolean;
  /** A quieter row: a done track, a split-off, an action. */
  quiet?: boolean;
} & ({ link: SectionLink; onSelect?: never } | { link?: never; onSelect: () => void });

type Group = { heading?: string; options: Option[] };

type Menu = {
  /** The menu's name ("Switch part"). */
  label: string;
  /** The find field's placeholder ("Find a part"). */
  find: string;
  groups: Group[];
  /** Actions after the siblings (New part, Start track); hidden while finding. */
  actions?: Option[];
  /** The section the siblings open on, said in the footer. */
  stay?: Section;
};

function WorkspaceMenu({ data, children }: { data: PathData; children: (menu: Menu) => ReactNode }) {
  const { workspace, workspaces, scope } = data;
  const section = scope.level === "workspace" && scope.section ? scope.section : "inbox";
  return children({
    label: "Switch workspace",
    find: "Find a workspace",
    groups: [
      {
        options: workspaces.map((candidate) => ({
          key: candidate.id,
          label: candidate.name,
          link: workspaceLink(candidate.id, section === "my-tracks" || section === "projects" ? section : "inbox"),
          current: candidate.id === workspace.id,
        })),
      },
    ],
  });
}

function ProjectMenu({ data, children }: { data: PathData; children: (menu: Menu) => ReactNode }) {
  const { workspace, projects, scope } = data;
  const { parts: allParts } = useWorkspaceParts(workspace.id, projects);
  const stay = projectSection(keptSection("project", scope));
  return children({
    label: "Switch project",
    find: "Find a project",
    stay,
    groups: [
      {
        options: (projects ?? []).map((project) => {
          const parts = allParts?.get(project.id);
          return {
            key: project.id,
            label: project.name,
            mark: <ProjectMark name={project.name} parts={markParts(parts)} />,
            end: parts ? `${parts.length} ${parts.length === 1 ? "part" : "parts"}` : undefined,
            link: projectLink(workspace.id, project.id, stay),
            current: project.id === scope.projectId,
          };
        }),
      },
    ],
  });
}

function PartMenu({ data, children }: { data: PathData; children: (menu: Menu) => ReactNode }) {
  const { workspace, project, parts, partId, scope, onCreate } = data;
  const projectId = scope.projectId ?? "";
  const kept = keptSection("part", scope);
  const stay = partSection(kept);
  const sorted = useMemo(() => {
    const tones = partTones(parts ?? []);
    return [...(parts ?? [])].sort((a, b) => (tones.get(a.id) ?? 0) - (tones.get(b.id) ?? 0)).map((part) => ({ part, tone: tones.get(part.id) ?? 1 }));
  }, [parts]);
  return children({
    label: "Switch part",
    find: "Find a part",
    stay,
    groups: [
      {
        options: [
          {
            key: "project-wide",
            label: "Project-wide",
            kind: "all parts",
            mark: <ProjectMark name={project?.name ?? ""} parts={markParts(parts)} />,
            link: projectLink(workspace.id, projectId, projectSection(kept)),
          },
        ],
      },
      {
        options: sorted.map(({ part, tone }) => ({
          key: part.id,
          label: part.name,
          kind: part.kind,
          mark: <PartIcon name={part.name} kind={part.kind} tone={tone} />,
          link: partLink(workspace.id, projectId, part.id, stay),
          current: part.id === partId,
        })),
      },
    ],
    actions: [{ key: "new-part", label: "New part", mark: <Plus aria-hidden className="size-(--icon-sm) text-faint" strokeWidth={2.25} />, onSelect: () => onCreate("part"), quiet: true }],
  });
}

function FeatureMenu({ data, children }: { data: PathData; children: (menu: Menu) => ReactNode }) {
  const { workspace, features, partId, part, scope } = data;
  const stay = featureSection(scope.section);
  return children({
    label: "Switch feature",
    find: part ? `Find a feature in ${part.name}` : "Find a feature",
    stay,
    groups: [
      {
        options: (features ?? []).map((feature) => ({
          key: feature.id,
          label: feature.name,
          mark: <Box aria-hidden strokeWidth={2.25} className="size-(--icon-sm) shrink-0 text-ink-3" />,
          link: featureLink(workspace.id, scope.projectId ?? "", partId ?? "", feature.id, stay),
          current: feature.id === scope.featureId,
        })),
      },
    ],
  });
}

function TrackMenu({ data, children }: { data: PathData; children: (menu: Menu) => ReactNode }) {
  const { workspace, home, parts, partId, part, track, scope, onCreate } = data;
  const projectId = scope.projectId ?? "";
  const stay = trackSection(scope.section);
  const all = home?.tracks ?? [];
  const row = (summary: TrackSummary, quiet = false): Option => ({
    key: summary.track.id,
    label: summary.track.name,
    mark: <StateIcon state={summary.track.state} />,
    link: trackLink(workspace.id, projectId, summary.track.id, stay),
    current: summary.track.id === scope.trackId,
    quiet,
  });
  const onPart = all.filter((summary) => summary.track.part_id === partId);
  const open = onPart.filter((summary) => inFlight(summary.track.state));
  const closed = onPart.filter((summary) => !inFlight(summary.track.state));
  // The tracks split off from this one, or that it split off from, on another part.
  const own = track?.track;
  const split = all.filter(
    (summary) =>
      summary.track.part_id !== partId &&
      own &&
      (summary.track.split_from_track_id === own.id || summary.track.id === own.split_from_track_id),
  );
  const partName = (id: string) => parts?.find((candidate) => candidate.id === id)?.name;
  return children({
    label: "Switch track",
    find: part ? `Find a track in ${part.name}` : "Find a track",
    stay,
    groups: [
      { heading: "In flight", options: open.map((summary) => row(summary)) },
      { heading: "Done and dropped", options: closed.map((summary) => row(summary, true)) },
      {
        options: split.map((summary) => ({
          ...row(summary, true),
          label: `${summary.track.split_from_track_id === own?.id ? "Split off" : "Split from"}: ${summary.track.name}`,
          kind: partName(summary.track.part_id),
        })),
      },
    ],
    actions: [{ key: "start-track", label: "Start track", mark: <Plus aria-hidden className="size-(--icon-sm) text-faint" strokeWidth={2.25} />, onSelect: () => onCreate("track"), quiet: true }],
  });
}

/** The menu of `level`'s siblings, handed to `children` to draw. */
function SiblingMenu({ level, data, children }: { level: Level; data: PathData; children: (menu: Menu) => ReactNode }) {
  switch (level) {
    case "workspace":
      return <WorkspaceMenu data={data}>{children}</WorkspaceMenu>;
    case "project":
      return <ProjectMenu data={data}>{children}</ProjectMenu>;
    case "part":
      return <PartMenu data={data}>{children}</PartMenu>;
    case "feature":
      return <FeatureMenu data={data}>{children}</FeatureMenu>;
    case "track":
      return <TrackMenu data={data}>{children}</TrackMenu>;
  }
}

const matching = (query: string, option: Option) => [option.label, option.kind].some((text) => text?.toLowerCase().includes(query));

const row =
  "flex h-8 w-full min-w-0 items-center gap-2.5 rounded-control px-2 text-left font-medium whitespace-nowrap text-ink outline-none transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover focus-visible:bg-hover data-current:bg-selected";
const phoneRow =
  "-mx-2 flex h-12 w-[calc(100%+16px)] min-w-0 items-center gap-3 rounded-card px-2 text-left text-[15px] font-medium text-ink outline-none active:bg-hover focus-visible:bg-hover data-current:bg-selected";

function OptionRow({ option, phone }: { option: Option; phone: boolean }) {
  const inner = (
    <>
      {option.mark}
      <span className={cn("min-w-0 truncate", option.quiet && "text-ink-2")}>{option.label}</span>
      {option.kind && <span className="shrink-0 text-meta font-normal text-ink-3">{option.kind}</span>}
      {(option.end || option.current) && (
        <span className="ml-auto flex shrink-0 items-center gap-2 pl-2 text-meta font-normal text-ink-3 tabular-nums">
          {option.end}
          {option.current && <Check aria-hidden className="size-(--icon-sm) text-ink" strokeWidth={2.25} />}
        </span>
      )}
    </>
  );
  const className = phone ? phoneRow : row;
  if (option.link) {
    return (
      <Link
        data-menu-opt
        data-current={option.current || undefined}
        {...option.link}
        activeOptions={{ exact: true }}
        activeProps={{}}
        aria-current={option.current ? "true" : undefined}
        className={className}
      >
        {inner}
      </Link>
    );
  }
  return (
    <button data-menu-opt type="button" onClick={option.onSelect} className={className}>
      {inner}
    </button>
  );
}

const Separator = ({ phone }: { phone?: boolean }) => <div aria-hidden className={cn("my-1 h-px bg-line", phone ? "" : "mx-2")} />;

/** The groups of a menu, narrowed to `query`. */
function MenuBody({ menu, query, phone }: { menu: Menu; query: string; phone: boolean }) {
  const wanted = query.trim().toLowerCase();
  const groups = menu.groups
    .map((group) => ({ ...group, options: wanted ? group.options.filter((option) => matching(wanted, option)) : group.options }))
    .filter((group) => group.options.length > 0);
  const actions = wanted ? [] : (menu.actions ?? []);
  return (
    <>
      {groups.map((group, at) => (
        <div key={group.heading ?? at} role="group" aria-label={group.heading}>
          {at > 0 && !group.heading && <Separator phone={phone} />}
          {group.heading && <div className={cn("flex h-7 items-center text-meta font-medium text-ink-3", phone ? "mt-1" : "px-2")}>{group.heading}</div>}
          {group.options.map((option) => (
            <OptionRow key={option.key} option={option} phone={phone} />
          ))}
        </div>
      ))}
      {wanted && groups.length === 0 && <p className="px-2 py-1.5 text-meta text-ink-3">No {menu.find.replace(/^Find an? /, "").replace(/ in .*/, "")} matches</p>}
      {actions.length > 0 && (
        <>
          {groups.length > 0 && <Separator phone={phone} />}
          {actions.map((option) => (
            <OptionRow key={option.key} option={option} phone={phone} />
          ))}
        </>
      )}
    </>
  );
}

function Stay({ section }: { section: Section }) {
  return (
    <span>
      You stay on <b className="font-medium text-ink-2">{SECTION_LABELS[section]}</b>
    </span>
  );
}

/** The menu as a popover under its crumb: a find field, the siblings, and what you stay on. ↑↓ move, ↵ opens, Esc closes. */
function MenuPopover({ menu, onClose }: { menu: Menu; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const root = useRef<HTMLDivElement>(null);

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const box = root.current;
    if (!box) return;
    const target = event.target as HTMLElement;
    const options = Array.from(box.querySelectorAll<HTMLElement>("[data-menu-opt]"));
    const inInput = target.matches("input");
    if (event.key === "Enter" && inInput) {
      if (options[0]) {
        event.preventDefault();
        options[0].click();
      }
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    if (inInput) {
      (options.find((option) => option.hasAttribute("data-current")) ?? options[0])?.focus();
      return;
    }
    const at = options.indexOf(target);
    if (event.key === "ArrowUp" && at <= 0) {
      box.querySelector<HTMLInputElement>("input")?.focus();
      return;
    }
    options[Math.min(options.length - 1, Math.max(0, at + (event.key === "ArrowDown" ? 1 : -1)))]?.focus();
  }

  return (
    <div
      ref={root}
      role="dialog"
      aria-label={menu.label}
      onKeyDown={onKeyDown}
      onClick={(event) => {
        if ((event.target as HTMLElement).closest("a, button[data-menu-opt]")) onClose();
      }}
      className="absolute top-[calc(100%+6px)] left-0 z-40 flex max-h-[min(520px,calc(100dvh-72px))] w-(--menu-w) max-w-[calc(100vw-24px)] origin-top-left animate-pop-in flex-col overflow-hidden rounded-sheet bg-sheet shadow-pop"
    >
      <label className="flex h-11 shrink-0 items-center gap-2.5 border-b border-line px-4">
        <Search aria-hidden className="size-(--icon-sm) shrink-0 text-faint" strokeWidth={2.25} />
        <input
          type="text"
          autoFocus
          aria-label={menu.find}
          placeholder={menu.find}
          autoComplete="off"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-ink-3"
        />
      </label>
      <div className="min-h-0 overflow-y-auto p-1.5">
        <MenuBody menu={menu} query={query} phone={false} />
      </div>
      {menu.stay && (
        <div className="flex h-9 shrink-0 items-center border-t border-line bg-frame px-4 text-meta text-ink-3">
          <Stay section={menu.stay} />
        </div>
      )}
    </div>
  );
}

/** One step of the path on a desktop: its name, a link to the level's first section, and the ⌄ that opens its siblings. */
function CrumbStep({ crumb, data, current }: { crumb: Crumb; data: PathData; current: boolean }) {
  const popover = usePopover();
  return (
    <div
      ref={popover.anchor}
      data-crumb={crumb.level}
      className={cn("relative flex min-w-0 items-center", current ? "shrink" : "shrink-[3]")}
    >
      <Link
        {...crumb.link}
        activeOptions={{ exact: true }}
        activeProps={{}}
        aria-current={current ? "location" : undefined}
        className={cn(
          "flex h-(--control-h-lg) min-w-0 items-center gap-2 rounded-control pr-1 pl-2 whitespace-nowrap",
          "transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none",
          current ? "font-semibold text-ink" : "font-medium text-ink-2 hover:text-ink",
        )}
      >
        {crumb.mark}
        <span className="min-w-0 truncate">{crumb.name || " "}</span>
      </Link>
      <button
        ref={popover.opener}
        type="button"
        aria-haspopup="dialog"
        aria-expanded={popover.open}
        aria-label={`Switch ${NOUN[crumb.level]}`}
        onClick={popover.toggle}
        className="grid h-[22px] w-5 shrink-0 place-items-center rounded-badge text-faint transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover hover:text-ink focus-visible:shadow-focus focus-visible:outline-none aria-expanded:bg-selected aria-expanded:text-ink"
      >
        <ChevronDown aria-hidden className="size-3.5" strokeWidth={2.25} />
      </button>
      {popover.open && <SiblingMenu level={crumb.level} data={data}>{(menu) => <MenuPopover menu={menu} onClose={() => popover.setOpen(false)} />}</SiblingMenu>}
    </div>
  );
}

/** The path on a desktop: its steps with a slash between each. */
export function PathCrumbs({ crumbs, data }: { crumbs: Crumb[]; data: PathData }) {
  return (
    <nav aria-label="Breadcrumb" className="flex min-w-0 flex-1 items-center">
      {crumbs.map((crumb, at) => (
        <div key={crumb.level} className={cn("flex min-w-0 items-center", at === crumbs.length - 1 ? "shrink" : "shrink-[3]")}>
          {at > 0 && (
            <span aria-hidden className="mx-0.5 shrink-0 text-[15px] text-line-strong select-none">
              /
            </span>
          )}
          <CrumbStep crumb={crumb} data={data} current={at === crumbs.length - 1} />
        </div>
      ))}
    </nav>
  );
}

/** The phone's sheet of the open level's siblings, with the path above them, each step a link up. */
export function SiblingsSheet({ crumbs, data, onClose }: { crumbs: Crumb[]; data: PathData; onClose: () => void }) {
  const current = crumbs[crumbs.length - 1];
  const above = crumbs.slice(0, -1);
  return (
    <SiblingMenu level={current.level} data={data}>
      {(menu) => (
        <BottomSheet label={menu.label} onClose={onClose}>
          {above.length > 0 && (
            <nav aria-label="Path" className="flex min-w-0 flex-wrap items-center gap-x-1 pt-1 pb-1.5 text-meta font-medium text-ink-3">
              {above.map((crumb, at) => (
                <span key={crumb.level} className="flex min-w-0 items-center gap-1">
                  {at > 0 && <span aria-hidden>/</span>}
                  <Link {...crumb.link} activeProps={{}} className="truncate text-ink-2 underline-offset-2 hover:underline">
                    {crumb.name}
                  </Link>
                </span>
              ))}
            </nav>
          )}
          <div className="pb-1">
            <MenuBody menu={menu} query="" phone />
          </div>
          {menu.stay && (
            <p className="pt-1 pb-1 text-meta text-ink-3">
              <Stay section={menu.stay} />
            </p>
          )}
        </BottomSheet>
      )}
    </SiblingMenu>
  );
}
