import { useEffect, useId, useState } from "react";
import { ArrowRight } from "lucide-react";
import { loadVersionTokens } from "@/api/review";
import { Badge } from "@/components/ui/badge";
import { diffTokens, type TokenChange, type TokenValues } from "@/design/token-diff";
import { cn } from "@/lib/utils";

// What a change set changes in the design system's tokens: the design-system
// doc's node changes say nothing about them, so the review loads the token
// tree of the base version and of the version under review and lists each
// added, changed and removed token with its value in every mode.

/** How many token changes are listed before "Show all". */
const FIRST = 20;

const BADGE = { added: "green", changed: "amber", removed: "red" } as const;
const SIGN = { added: "+", changed: "~", removed: "−" } as const;
const TONE = {
  added: "text-emerald-700 dark:text-emerald-400",
  changed: "text-amber-700 dark:text-amber-400",
  removed: "text-red-700 dark:text-red-400",
} as const;

/** A token value as text: strings as they are, anything else (a shadow, a typography) as compact JSON. */
function show(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value);
}

/** Whether a value is a colour that a swatch can show. */
function isColour(value: unknown): value is string {
  return typeof value === "string" && /^(#[0-9a-f]{3,8}|rgba?\(|hsla?\(|oklch\(|oklab\()/i.test(value.trim());
}

function Value({ value, old }: { value: unknown; old?: boolean }) {
  if (value === undefined) return <span className="text-muted-foreground/60">—</span>;
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1.5", old && "text-muted-foreground line-through decoration-muted-foreground/50")}>
      {isColour(value) && (
        <span aria-hidden className="size-3.5 shrink-0 rounded-[4px] ring-1 ring-black/10" style={{ background: value }} />
      )}
      <span className="truncate font-mono text-[12px]">{show(value)}</span>
    </span>
  );
}

/** The values to compare: one line per mode, or one plain line for a token without modes. */
function lines(before: TokenValues | null, after: TokenValues | null): { label: string | null; before: unknown; after: unknown }[] {
  const modes = [...new Set([...Object.keys(before?.modes ?? {}), ...Object.keys(after?.modes ?? {})])];
  if (!modes.length) return [{ label: null, before: before?.value, after: after?.value }];
  return modes.map((mode) => ({ label: mode, before: before?.modes[mode], after: after?.modes[mode] }));
}

function Row({ change }: { change: TokenChange }) {
  return (
    <li className="grid gap-x-4 gap-y-1.5 px-4 py-2.5 text-sm sm:grid-cols-[minmax(0,14rem)_minmax(0,1fr)]">
      <div className="flex min-w-0 items-center gap-2">
        <span className="truncate font-mono text-[12.5px] font-medium">{change.path}</span>{" "}
        <Badge variant={BADGE[change.change]}>{change.change}</Badge>
      </div>
      {" "}
      <div className="min-w-0 space-y-1">
        {lines(change.before, change.after).map((line) => (
          <div key={line.label ?? "value"} className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
            {line.label && <span className="w-14 shrink-0 text-xs text-muted-foreground">{line.label}</span>}{" "}
            {change.change !== "added" && <Value value={line.before} old={change.change === "changed"} />}{" "}
            {change.change === "changed" && <ArrowRight aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />}{" "}
            {change.change !== "removed" && <Value value={line.after} />}
          </div>
        ))}
      </div>
    </li>
  );
}

/** The design system's token changes from `baseVersionId` (null for a new design system) to `versionId`. */
export function TokenChanges({
  workspaceId,
  projectId,
  baseVersionId,
  versionId,
}: {
  workspaceId: string;
  projectId: string;
  baseVersionId: string | null;
  versionId: string;
}) {
  const headingId = useId();
  const [changes, setChanges] = useState<TokenChange[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [all, setAll] = useState(false);

  useEffect(() => {
    let current = true;
    setChanges(null);
    setFailed(false);
    Promise.all([
      baseVersionId ? loadVersionTokens(workspaceId, projectId, baseVersionId) : Promise.resolve(null),
      loadVersionTokens(workspaceId, projectId, versionId),
    ]).then(
      ([base, next]) => {
        if (current) setChanges(diffTokens(base?.tokens ?? null, next.tokens));
      },
      () => {
        if (current) setFailed(true);
      },
    );
    return () => {
      current = false;
    };
  }, [workspaceId, projectId, baseVersionId, versionId]);

  const counts = (["added", "changed", "removed"] as const).map((kind) => ({
    kind,
    count: changes?.filter((change) => change.change === kind).length ?? 0,
  }));
  const shown = changes && !all ? changes.slice(0, FIRST) : changes;

  return (
    <section aria-labelledby={headingId} className="overflow-hidden rounded-lg border">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 border-b bg-muted/40 px-4 py-2">
        <h4 id={headingId} className="text-[13px] font-semibold">
          Tokens
        </h4>
        {changes && (
          <p className="flex flex-wrap items-center gap-x-3 text-xs text-muted-foreground">
            {counts.map(({ kind, count }) => (
              <span key={kind} className={cn("inline-flex items-baseline gap-1", count === 0 && "opacity-50")}>
                <span className={cn("font-mono text-[13px] font-medium tabular-nums", count > 0 && TONE[kind])}>
                  {SIGN[kind]}
                  {count}
                </span>{" "}
                {kind}
              </span>
            ))}
          </p>
        )}
      </div>
      {failed && <p className="px-4 py-3 text-sm text-destructive">The design tokens could not be loaded.</p>}
      {!changes && !failed && <p className="px-4 py-3 text-sm text-muted-foreground">Comparing the design tokens…</p>}
      {changes && changes.length === 0 && (
        <p className="px-4 py-3 text-sm text-muted-foreground">No token changes.</p>
      )}
      {shown && shown.length > 0 && (
        <ul className="divide-y">
          {shown.map((change) => (
            <Row key={change.path} change={change} />
          ))}
        </ul>
      )}
      {changes && !all && changes.length > FIRST && (
        <button
          type="button"
          onClick={() => setAll(true)}
          className="w-full border-t px-4 py-2 text-left text-xs font-medium text-muted-foreground hover:bg-accent/60 hover:text-foreground"
        >
          Show all {changes.length} token changes
        </button>
      )}
    </section>
  );
}
