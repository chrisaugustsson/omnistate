import { useState, type ReactNode } from "react";
import { Check, Ellipsis } from "lucide-react";
import type { LinkedBody, StoryBody } from "@/api/generated";
import { Drawer, DialogError } from "@/components/shell/dialog";
import { usePopover } from "@/components/shell/popover";
import { StateIcon, stateLabel } from "@/components/shell/state-icon";
import { MenuSeparator, buttonClass, menuClass, menuItemClass } from "@/components/track/ui";
import { cn } from "@/lib/utils";
import type { Steer } from "./board";

// A story opened from its card: a drawer at the right edge (the whole screen
// on the phone) with its key,
// status and priority over its title, then its text, its criteria with their
// checkboxes, what blocks it and its open findings. A criterion an agent
// ticked that waits on a human check is the one checkbox a reader can tick;
// the rest show where the agents are. The "…" menu sets the priority and
// cancels the story, after asking.

function Section({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section aria-label={title} className="[&+&]:mt-6">
      <h3 className="mb-1.5 flex items-center gap-2 font-semibold text-ink">
        {title}
        {aside && <span className="font-medium text-ink-3 tabular-nums">{aside}</span>}
      </h3>
      {children}
    </section>
  );
}

const closed = (status: string) => status === "done" || status === "cancelled";

export function StoryDrawer({
  story,
  byId,
  linked,
  steer,
  full = false,
  onClose,
}: {
  story: StoryBody;
  byId: Map<string, StoryBody>;
  linked: LinkedBody[];
  steer: Steer;
  /** Fills the screen (on the phone). */
  full?: boolean;
  onClose: () => void;
}) {
  const menu = usePopover();
  const [confirming, setConfirming] = useState(false);
  const isClosed = closed(story.status);
  const holder = story.holder as { run?: string; stage?: string } | null | undefined;
  const ticked = story.criteria.filter((criterion) => criterion.ticked && !criterion.pending).length;
  const findings = story.findings.filter((finding) => finding.resolved !== true) as { stage?: string; text?: string }[];
  const message = steer.messages.get(story.id);
  const write = (action: () => Promise<StoryBody>) => steer.run(story, action);

  const headActions = !isClosed && (
    <div ref={menu.anchor} className="relative">
      <button
        ref={menu.opener}
        type="button"
        aria-label={`${story.key} actions`}
        aria-haspopup="menu"
        aria-expanded={menu.open}
        onClick={menu.toggle}
        className={buttonClass({ variant: "ghost", icon: true })}
      >
        <Ellipsis aria-hidden strokeWidth={2.25} />
      </button>
      {menu.open && (
        <div role="menu" aria-label={`${story.key} actions`} className={cn(menuClass, "min-w-[180px]")}>
          {story.priority !== 0 && (
            <>
              <div className="flex h-7 items-center px-2 text-meta text-ink-3">Priority</div>
              {[1, 2, 3].map((value) => (
                <button
                  key={value}
                  type="button"
                  role="menuitemradio"
                  aria-checked={story.priority === value}
                  onClick={() => {
                    menu.setOpen(false);
                    if (story.priority !== value) void write(() => steer.patch(story, { priority: value }));
                  }}
                  className={menuItemClass}
                >
                  <span className="grid w-4 place-items-center">{story.priority === value && <Check aria-hidden className="size-(--icon-sm)" />}</span>
                  {`P${value}`}
                </button>
              ))}
              <MenuSeparator />
            </>
          )}
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              menu.setOpen(false);
              setConfirming(true);
            }}
            className={cn(menuItemClass, "text-danger-ink")}
          >
            Cancel story…
          </button>
        </div>
      )}
    </div>
  );

  return (
    <Drawer
      full={full}
      title={story.title}
      eyebrow={
        <>
          <span className="font-mono text-[12px] text-ink-2 tabular-nums">{story.key}</span>
          <span className="inline-flex items-center gap-1.5 font-medium text-ink-2">
            <StateIcon state={story.blocked && !isClosed ? "blocked" : story.status} />
            {story.blocked && !isClosed ? `${stateLabel(story.status)} · blocked` : stateLabel(story.status)}
          </span>
          <span
            title={story.priority === 0 ? "Priority 0: a fix" : `Priority ${story.priority}`}
            className="inline-flex h-5 items-center rounded-badge bg-hover px-1.5 font-medium text-ink-2 tabular-nums"
          >
            P{story.priority}
          </span>
          {holder?.stage && (
            <span>
              {holder.stage}
              {holder.run && ` · run ${holder.run}`}
            </span>
          )}
        </>
      }
      headActions={headActions}
      onClose={onClose}
      footerStart={message && <DialogError>{message}</DialogError>}
      footer={
        confirming && !isClosed ? (
          <>
            <span className="mr-auto font-medium text-ink">{`Cancel ${story.key}?`}</span>
            <button type="button" className={buttonClass()} onClick={() => setConfirming(false)}>
              Keep
            </button>
            <button
              type="button"
              className={buttonClass({ variant: "danger" })}
              onClick={async () => {
                setConfirming(false);
                await write(() => steer.cancel(story));
              }}
            >
              Cancel story
            </button>
          </>
        ) : (
          <button type="button" className={buttonClass()} onClick={onClose}>
            Close
          </button>
        )
      }
    >
      {story.text.trim() ? (
        <p className="text-em leading-(--lh-prose) whitespace-pre-line text-ink">{story.text.trim()}</p>
      ) : (
        <p className="text-ink-3">No text yet.</p>
      )}
      {story.topic && <p className="mt-2 text-meta text-ink-3">Topic: {story.topic}</p>}

      <div className="mt-6">
        <Section title="Criteria" aside={story.criteria.length > 0 ? `${ticked} of ${story.criteria.length}` : undefined}>
          {story.criteria.length === 0 ? (
            <p className="text-ink-3">None written.</p>
          ) : (
            <ul className="-mx-2">
              {story.criteria.map((criterion, index) => {
                const waiting = criterion.ticked && criterion.pending;
                return (
                  <li key={index}>
                    <label className={cn("flex items-start gap-2.5 rounded-control px-2 py-1.5", waiting && "cursor-pointer hover:bg-hover")}>
                      <input
                        type="checkbox"
                        checked={criterion.ticked && !criterion.pending}
                        disabled={!waiting}
                        onChange={() => {
                          if (waiting) void write(() => steer.check(story, index));
                        }}
                        className="mt-0.5 size-3.5 shrink-0 accent-(--brand) disabled:opacity-60"
                      />
                      <span className="min-w-0 flex-1">
                        <span className={cn("text-ink", criterion.ticked && !criterion.pending && "text-ink-2")}>{criterion.text}</span>
                        {waiting ? (
                          <span className="block text-meta font-medium text-brand-ink">Ticked by an agent; tick it once you have checked it</span>
                        ) : (
                          criterion.human_check && <span className="block text-meta text-ink-3">Human check</span>
                        )}
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
          )}
        </Section>

        {story.blocked_by.length > 0 && (
          <Section title="Blocked by" aside={story.blocked_by.length}>
            <ul className="space-y-1">
              {story.blocked_by.map((id) => {
                const here = byId.get(id);
                const there = story.blockers?.find((other) => other.id === id);
                const task = linked.find((one) => one.id === id);
                const status = here?.status ?? there?.status ?? task?.status ?? "";
                const finished = closed(status);
                return (
                  <li key={id} className="flex min-w-0 items-start gap-2">
                    <StateIcon state={status || "backlog"} className="mt-0.5" />
                    <span className="min-w-0 flex-1">
                      <span className={cn("font-mono text-[12px] text-ink-2", finished && "line-through decoration-ink-3")}>{here?.key ?? there?.key ?? task?.key ?? "?"}</span>{" "}
                      <span className="text-ink">{here?.title ?? task?.title ?? ""}</span>
                      {!here && (task || there) && (
                        <span className="block text-meta text-ink-3">
                          {[task?.part_name, there?.track_name ?? task?.track_name].filter(Boolean).join(" · ")}
                          {(there?.blocker_dropped || task?.blocker_dropped) && " · track dropped"}
                        </span>
                      )}
                    </span>
                    <span className="shrink-0 text-meta text-ink-3">{status && stateLabel(status)}</span>
                  </li>
                );
              })}
            </ul>
          </Section>
        )}

        {findings.length > 0 && (
          <Section title="Open findings" aside={findings.length}>
            <ul className="space-y-2">
              {findings.map((finding, index) => (
                <li key={index} className="border-l-2 border-line pl-3">
                  {finding.stage && <span className="block text-meta text-ink-3">{finding.stage}</span>}
                  <span className="whitespace-pre-line text-ink">{finding.text}</span>
                </li>
              ))}
            </ul>
          </Section>
        )}
      </div>
    </Drawer>
  );
}
