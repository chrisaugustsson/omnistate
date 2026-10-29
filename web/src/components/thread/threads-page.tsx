import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { Check, ChevronDown } from "lucide-react";
import { Empty, List } from "@/components/project/items";
import { SectionPage } from "@/components/project/section-page";
import { ThreadRow, threadTime } from "@/components/project/scope-page";
import { cn } from "@/lib/utils";

// The Threads page: the threads of every scope of the project, newest post
// first, with a filter that narrows them to the app level or one feature. The
// choice is the page's `scope` search parameter (`app` or a feature's ID);
// without it every scope is listed. The list is the project's home, which is
// loaded again on every live event, so threads reorder and their counts and
// flags change as agents post.

type ScopeOption = { value: string | undefined; label: string };

/** A button that opens a searchable list of scopes: All scopes, App and each feature by name. */
function ScopeFilter({
  options,
  value,
  onChange,
}: {
  options: ScopeOption[];
  value: string | undefined;
  onChange: (value: string | undefined) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const listId = useId();
  const shown = useMemo(
    () => options.filter((option) => option.label.toLowerCase().includes(query.trim().toLowerCase())),
    [options, query],
  );
  const current = options.find((option) => option.value === value);

  useEffect(() => {
    if (!open) return;
    const away = (event: MouseEvent) => {
      if (root.current && !root.current.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);

  function choose(option: ScopeOption) {
    setOpen(false);
    setQuery("");
    onChange(option.value);
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive((index) => Math.min(index + 1, shown.length - 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive((index) => Math.max(index - 1, 0));
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (shown[active]) choose(shown[active]);
    } else if (event.key === "Escape") {
      setOpen(false);
    }
  }

  return (
    <div ref={root} className="relative w-64 max-w-full">
      <button
        type="button"
        aria-label="Scope"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => {
          setOpen((now) => !now);
          setQuery("");
          setActive(0);
        }}
        className="flex h-9 w-full items-center justify-between gap-2 rounded-md border border-input bg-background px-3 text-sm shadow-xs transition-colors outline-none hover:bg-accent/60 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/30"
      >
        <span className="truncate">{current?.label ?? "Feature"}</span>
        <ChevronDown aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
      </button>
      {open && (
        <div className="absolute z-20 mt-1 w-full overflow-hidden rounded-md border bg-popover text-popover-foreground shadow-panel">
          <input
            autoFocus
            role="combobox"
            aria-label="Search scopes"
            aria-expanded
            aria-controls={listId}
            aria-activedescendant={shown[active] ? `${listId}-${active}` : undefined}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
            onKeyDown={onKeyDown}
            placeholder="Search scopes"
            className="block h-9 w-full border-b bg-transparent px-3 text-sm outline-none placeholder:text-muted-foreground"
          />
          <ul id={listId} role="listbox" aria-label="Scopes" className="max-h-64 overflow-y-auto py-1">
            {shown.length === 0 && <li className="px-3 py-2 text-sm text-muted-foreground">No scope found</li>}
            {shown.map((option, index) => (
              <li
                key={option.value ?? "all"}
                id={`${listId}-${index}`}
                role="option"
                aria-selected={option.value === value}
                onMouseEnter={() => setActive(index)}
                onClick={() => choose(option)}
                className={cn(
                  "flex cursor-pointer items-center gap-2 px-3 py-1.5 text-sm",
                  index === active && "bg-accent",
                )}
              >
                <span className="min-w-0 flex-1 truncate">{option.label}</span>
                {option.value === value && <Check aria-hidden className="size-3.5 shrink-0" />}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

/** The project's threads, newest post first; `scope` is `app`, a feature's ID, or absent for every scope. */
export function ThreadsPage({
  workspaceId,
  projectId,
  scope,
  onScope,
}: {
  workspaceId: string;
  projectId: string;
  scope: string | undefined;
  onScope: (scope: string | undefined) => void;
}) {
  const params = { workspaceId, projectId };
  return (
    <SectionPage
      workspaceId={workspaceId}
      projectId={projectId}
      title="Threads"
      description={() => "Each thread is one question that agents work out and you follow."}
    >
      {(home) => {
        const names = new Map((home.features ?? []).map(({ feature }) => [feature.id, feature.name]));
        const scopeName = (featureId: string | null | undefined) =>
          featureId ? (names.get(featureId) ?? "Feature") : "App";
        const options: ScopeOption[] = [
          { value: undefined, label: "All scopes" },
          { value: "app", label: "App" },
          ...[...names].map(([id, name]) => ({ value: id, label: name })),
        ];
        const shown = home.threads
          .filter((thread) => scope === undefined || (scope === "app" ? !thread.feature_id : thread.feature_id === scope))
          .sort((a, b) => threadTime(b).localeCompare(threadTime(a)));
        return (
          <>
            <ScopeFilter options={options} value={scope} onChange={onScope} />
            {shown.length === 0 ? (
              <Empty>No threads here yet.</Empty>
            ) : (
              <List>
                {shown.map((thread) => (
                  <ThreadRow key={thread.id} thread={thread} params={params} scopeName={scopeName(thread.feature_id)} eachFlag />
                ))}
              </List>
            )}
          </>
        );
      }}
    </SectionPage>
  );
}
