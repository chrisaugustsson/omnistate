import { useState, type ReactNode } from "react";
import { Check, X } from "lucide-react";
import { approveDecision, rejectDecision } from "@/api/threads";
import type { DecisionBody, DecisionOption, PostBody } from "@/api/generated";
import { AgentAvatar, AgentChip } from "@/components/agent-chip";
import { StatusBadge } from "@/components/status-badge";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { authorName, timeAgo } from "@/lib/format";
import { cn } from "@/lib/utils";
import { PostTypeBadge } from "./timeline";

const TEXTAREA_CLASS =
  "min-h-20 w-full rounded-lg border border-input bg-background px-3 py-2 text-[13px] leading-relaxed shadow-xs outline-none placeholder:text-muted-foreground/70 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/30";

/** A small uppercase heading over a group of the decision's fields. */
function Heading({ children }: { children: ReactNode }) {
  return <h3 className="text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">{children}</h3>;
}

function Shell({ status, at, children }: { status?: string; at?: string; children: ReactNode }) {
  return (
    <section
      aria-labelledby="decision"
      className="flex flex-col overflow-hidden rounded-xl border bg-card shadow-panel lg:max-h-[calc(100dvh-5rem)]"
    >
      <div className="flex items-center gap-2 border-b px-4 py-3">
        <h2 id="decision" className="text-[13px] font-semibold">
          Decision
        </h2>
        {status && <StatusBadge status={status} />}
        {at && <span className="ml-auto text-[11px] text-muted-foreground">{timeAgo(at)}</span>}
      </div>
      {children}
    </section>
  );
}

/** The current decision of the thread, with Approve and Reject while it is proposed. */
export function DecisionPanel({
  workspaceId,
  projectId,
  decision,
  posts,
  onChanged,
}: {
  workspaceId: string;
  projectId: string;
  decision: DecisionBody | null;
  posts: PostBody[];
  onChanged: () => void;
}) {
  const [rejecting, setRejecting] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  if (!decision) {
    return (
      <Shell>
        <p className="px-4 py-6 text-[13px] text-muted-foreground">No decision has been proposed yet.</p>
      </Shell>
    );
  }

  async function act(call: () => Promise<unknown>) {
    setBusy(true);
    setFailed(false);
    try {
      await call();
      setRejecting(false);
      setReason("");
      onChanged();
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }

  const current = decision;
  const relied = current.posts.map((id) => posts.find((post) => post.id === id)).filter((post) => post !== undefined);
  const scale = scoreScale(current.options);
  return (
    <Shell status={current.status} at={current.created_at}>
      <div className="min-h-0 flex-1 space-y-6 overflow-y-auto px-4 py-4 text-[13px]">
        {failed && (
          <Alert variant="destructive">
            <AlertDescription>The decision could not be changed. Reload and try again.</AlertDescription>
          </Alert>
        )}
        <p className="text-[17px] leading-snug font-medium tracking-[-0.01em] text-foreground">{current.direction}</p>
        <dl className="grid grid-cols-[6.5rem_minmax(0,1fr)] gap-x-3 gap-y-2.5">
          <dt className="text-muted-foreground">Chosen option</dt>
          <dd className="flex min-w-0 items-center gap-1.5 font-medium">
            <Check className="size-3.5 shrink-0 text-signal" />
            <span>{current.chosen}</span>
          </dd>
          <dt className="text-muted-foreground">Proposed by</dt>
          <dd className="min-w-0">
            <AgentChip name={authorName(current.author)} />
          </dd>
          <dt className="text-muted-foreground">Criteria</dt>
          <dd className="min-w-0">
            <ol className="space-y-1">
              {current.criteria.map((criterion, index) => (
                <li key={criterion} className="flex gap-1.5 leading-snug">
                  <span className="w-3 shrink-0 font-mono text-[11px] leading-[18px] text-muted-foreground tabular-nums">
                    {index + 1}
                  </span>
                  <span>{criterion}</span>
                </li>
              ))}
            </ol>
          </dd>
        </dl>
        {current.options.length > 0 && (
          <div className="space-y-2">
            <Heading>Options</Heading>
            <ul className="space-y-2">
              {current.options.map((option) => (
                <OptionCard
                  key={option.name}
                  option={option}
                  chosen={option.name === current.chosen}
                  criteria={current.criteria}
                  scale={scale}
                />
              ))}
            </ul>
          </div>
        )}
        <TextList title="Assumptions" items={current.assumptions} />
        <TextList title="Open questions" items={current.open_questions} />
        {current.dissent.length > 0 && (
          <div className="space-y-2">
            <Heading>Dissent</Heading>
            <ul className="space-y-2.5">
              {current.dissent.map((entry, index) => (
                <li key={index} className="space-y-1">
                  <span className="inline-flex items-center gap-1.5">
                    <AgentAvatar name={entry.by.split("/").pop() || entry.by} />
                    <span className="font-mono text-[11px] text-foreground/80">{entry.by}</span>
                  </span>
                  <p className="border-l-2 pl-2.5 leading-relaxed text-muted-foreground">{entry.text}</p>
                </li>
              ))}
            </ul>
          </div>
        )}
        {relied.length > 0 && (
          <div className="space-y-2">
            <Heading>Relies on</Heading>
            <ul className="divide-y overflow-hidden rounded-lg border">
              {relied.map((post) => (
                <li key={post.id} className="flex items-start gap-2 px-2.5 py-2">
                  <AgentAvatar name={authorName(post.author)} className="mt-px" />
                  <span className="min-w-0 flex-1 leading-snug">{post.summary}</span>
                  <PostTypeBadge type={post.type} />
                </li>
              ))}
            </ul>
          </div>
        )}
        {current.rejection_reason && (
          <div className="space-y-1 rounded-lg bg-red-50 px-3 py-2.5 ring-1 ring-red-600/15 ring-inset dark:bg-red-400/10">
            <p className="text-[11px] font-medium tracking-[0.08em] text-red-800 uppercase dark:text-red-300">
              Reason for rejecting
            </p>
            <p className="leading-relaxed text-foreground">{current.rejection_reason}</p>
          </div>
        )}
      </div>
      {current.status === "proposed" && (
        <div className="space-y-3 border-t bg-secondary/40 px-4 py-3">
          {rejecting && (
            <div className="space-y-1.5">
              <label htmlFor="reject-reason" className="text-[12px] font-medium">
                Reason
              </label>
              <textarea
                id="reject-reason"
                className={TEXTAREA_CLASS}
                placeholder="What should the agents change?"
                value={reason}
                onChange={(event) => setReason(event.target.value)}
              />
            </div>
          )}
          <div className="flex gap-2">
            <Button
              className="flex-1"
              disabled={busy}
              onClick={() => act(() => approveDecision(workspaceId, projectId, current.id))}
            >
              <Check className="size-4" />
              Approve
            </Button>
            {rejecting ? (
              <>
                <Button
                  variant="destructive"
                  className="flex-1"
                  disabled={busy || !reason.trim()}
                  onClick={() => act(() => rejectDecision(workspaceId, projectId, current.id, reason.trim()))}
                >
                  Reject
                </Button>
                <Button variant="ghost" onClick={() => setRejecting(false)}>
                  Cancel
                </Button>
              </>
            ) : (
              <Button variant="outline" className="flex-1" disabled={busy} onClick={() => setRejecting(true)}>
                <X className="size-4" />
                Reject
              </Button>
            )}
          </div>
        </div>
      )}
    </Shell>
  );
}

/** The top of the score scale: 10 when every score fits in 0–10, else the highest score. */
function scoreScale(options: DecisionOption[]): number {
  const numbers = options.flatMap((option) => Object.values(option.scores).filter((v): v is number => typeof v === "number"));
  const highest = Math.max(0, ...numbers);
  return highest <= 10 ? 10 : highest;
}

function OptionCard({
  option,
  chosen,
  criteria,
  scale,
}: {
  option: DecisionOption;
  chosen: boolean;
  criteria: string[];
  scale: number;
}) {
  // The scores in the order of the criteria, then any the criteria do not name.
  const keys = [
    ...criteria.filter((criterion) => criterion in option.scores),
    ...Object.keys(option.scores).filter((criterion) => !criteria.includes(criterion)),
  ];
  return (
    <li
      className={cn(
        "space-y-2 rounded-lg border px-3 py-2.5",
        chosen ? "border-signal/40 bg-signal-soft/40 ring-2 ring-signal/15" : "bg-background",
      )}
    >
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 font-medium text-foreground">{option.name}</span>
        {chosen && (
          <Badge variant="signal" className="px-1.5 py-0 text-[11px] leading-[18px]">
            <Check />
            Chosen
          </Badge>
        )}
      </div>
      {keys.length > 0 && (
        <div className="space-y-1">
          {keys.map((criterion) => {
            const score = option.scores[criterion];
            const value = typeof score === "number" ? score : null;
            const index = criteria.indexOf(criterion);
            return (
              <div key={criterion} className="flex items-center gap-2 text-[12px]" title={criterion}>
                <span className="w-3 shrink-0 font-mono text-[11px] text-muted-foreground tabular-nums">
                  {index >= 0 ? index + 1 : "·"}
                </span>
                <span className="min-w-0 flex-1 truncate text-muted-foreground">{criterion}</span>
                {value !== null && (
                  <span aria-hidden className="h-1 w-14 shrink-0 overflow-hidden rounded-full bg-border">
                    <span
                      className={cn("block h-full rounded-full", chosen ? "bg-foreground/80" : "bg-foreground/30")}
                      style={{ width: `${Math.max(0, Math.min(100, (value / scale) * 100))}%` }}
                    />
                  </span>
                )}
                <span className="w-5 shrink-0 text-right font-mono text-[12px] text-foreground tabular-nums">
                  {String(score)}
                </span>
              </div>
            );
          })}
        </div>
      )}
      {option.lost_because && (
        <p className="leading-snug text-muted-foreground">
          <span className="font-medium text-foreground/70">Lost because: </span>
          {option.lost_because}
        </p>
      )}
    </li>
  );
}

function TextList({ title, items }: { title: string; items: string[] }) {
  if (!items.length) return null;
  return (
    <div className="space-y-2">
      <Heading>{title}</Heading>
      <ul className="space-y-1.5">
        {items.map((item, index) => (
          <li key={index} className="flex gap-2 leading-relaxed">
            <span aria-hidden className="mt-[0.6em] size-1 shrink-0 rounded-full bg-muted-foreground/50" />
            <span>{item}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
