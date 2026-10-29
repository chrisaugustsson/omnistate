import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { Link, useNavigate } from "@tanstack/react-router";
import { FileText, Layers, LayoutGrid, MessagesSquare, Search, type LucideIcon } from "lucide-react";
import type { HomeResponse } from "@/api/generated";
import { buildIndex, recentPlaces, search, type Place, type PlaceKind } from "@/components/palette/search";
import { cn } from "@/lib/utils";
import { useVisits } from "@/lib/visits";

// The ⌘K palette: finds a feature, doc or thread of the project by name or
// title, and with nothing typed lists the places visited last. It is a
// combobox over the project home; the search itself is in ./search, and
// no fuzzy matching is done here.

type Params = { workspaceId: string; projectId: string };

const ICONS: Record<PlaceKind, LucideIcon> = { app: LayoutGrid, feature: Layers, doc: FileText, thread: MessagesSquare };

/** The `to` and `params` that open a place. */
function linkOf(place: Place, params: Params) {
  switch (place.kind) {
    case "app":
      return { to: "/w/$workspaceId/projects/$projectId/app", params } as const;
    case "feature":
      return { to: "/w/$workspaceId/projects/$projectId/features/$featureId", params: { ...params, featureId: place.id } } as const;
    case "doc":
      return { to: "/w/$workspaceId/projects/$projectId/docs/$docId", params: { ...params, docId: place.id } } as const;
    case "thread":
      return { to: "/w/$workspaceId/projects/$projectId/threads/$threadId", params: { ...params, threadId: place.id } } as const;
  }
}

/** Whether this is a Mac, where the shortcut is ⌘K and not Ctrl+K. */
export function isMac(): boolean {
  return typeof navigator !== "undefined" && /mac|iphone|ipad/i.test(navigator.platform || navigator.userAgent);
}

/** Calls `toggle` for ⌘K (Ctrl+K off macOS), anywhere on the page. */
export function useSearchShortcut(toggle: () => void) {
  useEffect(() => {
    function onKey(event: globalThis.KeyboardEvent) {
      const modifier = isMac() ? event.metaKey : event.ctrlKey;
      if (modifier && event.key.toLowerCase() === "k") {
        event.preventDefault();
        toggle();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [toggle]);
}

type Group = { label: string; places: Place[]; count?: string };

function Row({ place, id, active, params, onOpen, onHover }: {
  place: Place;
  id: string;
  active: boolean;
  params: Params;
  onOpen: () => void;
  onHover: () => void;
}) {
  const Icon = ICONS[place.kind];
  return (
    <Link
      {...linkOf(place, params)}
      id={id}
      role="option"
      aria-selected={active}
      onClick={onOpen}
      onMouseMove={onHover}
      className={cn(
        "flex h-9 items-center gap-2.5 rounded-md px-2.5 text-sm outline-none",
        active ? "bg-accent text-foreground" : "text-foreground/90",
      )}
    >
      <Icon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1 truncate">{place.name}</span>
      {place.context && <span className="max-w-[45%] shrink-0 truncate text-xs text-muted-foreground">{place.context}</span>}
    </Link>
  );
}

function PaletteDialog({ home, params, onClose }: { home: HomeResponse; params: Params; onClose: () => void }) {
  const navigate = useNavigate();
  const visits = useVisits(params.projectId);
  const index = useMemo(() => buildIndex(home), [home]);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const listId = useId();
  const input = useRef<HTMLInputElement>(null);
  const typed = query.trim() !== "";

  const groups: Group[] = useMemo(() => {
    if (!typed) {
      const recent = recentPlaces(index, visits);
      return recent.length > 0 ? [{ label: "Recent", places: recent }] : [];
    }
    const { features, docs, threads } = search(index, query);
    const of = (group: { items: Place[]; total: number }) =>
      group.total > group.items.length ? `${group.items.length} of ${group.total}` : undefined;
    return [
      { label: "Features", places: features.items, count: of(features) },
      { label: "Docs", places: docs.items, count: of(docs) },
      { label: "Threads", places: threads.items, count: of(threads) },
    ].filter((group) => group.places.length > 0);
  }, [typed, index, visits, query]);
  const flat = groups.flatMap((group) => group.places);
  const current = Math.min(active, flat.length - 1);

  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    input.current?.focus();
    return () => before?.focus?.();
  }, []);

  useEffect(() => {
    if (current >= 0) document.getElementById(`${listId}-${current}`)?.scrollIntoView?.({ block: "nearest" });
  }, [current, listId]);

  function onKeyDown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (flat.length === 0) return;
      const step = event.key === "ArrowDown" ? 1 : -1;
      setActive((current + step + flat.length) % flat.length);
    } else if (event.key === "Enter" && flat[current]) {
      event.preventDefault();
      void navigate(linkOf(flat[current], params));
      onClose();
    }
  }

  let position = 0;
  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-foreground/20 px-3 pt-[12dvh] backdrop-blur-[1px]"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Search"
        onKeyDown={onKeyDown}
        className="flex max-h-[70dvh] w-full max-w-xl flex-col overflow-hidden rounded-xl border bg-popover shadow-2xl"
      >
        <label className="flex h-12 shrink-0 items-center gap-2.5 border-b px-3.5">
          <Search aria-hidden className="size-4 shrink-0 text-muted-foreground" />
          <input
            ref={input}
            type="text"
            role="combobox"
            aria-expanded
            aria-controls={listId}
            aria-activedescendant={current >= 0 ? `${listId}-${current}` : undefined}
            aria-label="Search"
            placeholder="Search features, docs and threads"
            autoComplete="off"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
            className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
          <kbd className="rounded border px-1.5 text-[11px] text-muted-foreground">esc</kbd>
        </label>
        <div id={listId} role="listbox" aria-label={typed ? "Results" : "Recent"} className="min-h-0 flex-1 overflow-y-auto p-1.5">
          {flat.length === 0 && (
            <p className="px-3 py-8 text-center text-sm text-muted-foreground">
              {typed
                ? `Nothing is called “${query.trim()}”. The palette searches names and titles, not what docs say.`
                : "No places visited yet. Type to search features, docs and threads."}
            </p>
          )}
          {groups.map((group) => (
            <div key={group.label} role="group" aria-label={group.label}>
              <div className="flex items-center justify-between px-2.5 pt-2 pb-1 text-[11px] font-medium text-muted-foreground">
                <span>{group.label}</span>
                {group.count && <span>{group.count}</span>}
              </div>
              {group.places.map((place) => {
                const at = position++;
                return (
                  <Row
                    key={`${place.kind}:${place.id}`}
                    place={place}
                    id={`${listId}-${at}`}
                    active={at === current}
                    params={params}
                    onOpen={onClose}
                    onHover={() => setActive(at)}
                  />
                );
              })}
            </div>
          ))}
        </div>
        <div className="flex shrink-0 items-center gap-4 border-t px-3.5 py-2 text-[11px] text-muted-foreground">
          <span>↑↓ move</span>
          <span>↵ open</span>
          <span>esc close</span>
          <span className="ml-auto hidden sm:inline">Names and titles only</span>
        </div>
      </div>
    </div>
  );
}

/** The palette, shown while `open`; it searches `home`, the project's items. */
export function Palette({ home, workspaceId, projectId, open, onClose }: Params & { home: HomeResponse | null; open: boolean; onClose: () => void }) {
  if (!open || !home) return null;
  return createPortal(<PaletteDialog home={home} params={{ workspaceId, projectId }} onClose={onClose} />, document.body);
}
