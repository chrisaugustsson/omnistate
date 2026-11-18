import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronRight, CircleCheck, MessageSquare, User } from "lucide-react";
import type { MemberBody, PartBody, ProjectBody } from "@/api/generated";
import { replyToComment } from "@/api/docs";
import { Avatar } from "@/components/shell/avatar";
import { PartIcon, ProjectMark } from "@/components/shell/part-icon";
import { docLink, workspaceLink } from "@/components/shell/scope";
import { StateIcon } from "@/components/shell/state-icon";
import { Kbd } from "@/components/shell/kbd";
import { ElementSnapshot } from "@/components/viewer/comments";
import { isMac } from "@/components/palette/palette";
import { partTone } from "@/lib/identity";
import { useEveryone } from "@/lib/inbox-scope";
import { cn } from "@/lib/utils";
import { age, firstLine, memberName, waitingGroups, whoOf, type WaitingGroup, type WaitingItem } from "./model";
import { buttonClass, trackLink } from "./rows";
import { findPart, useHomeData } from "./use-home-data";

// Home's Inbox (tasks/ui-redesign/v2/inbox.html): the list of what waits on
// you on the left, grouped by track, and the open item on the right, where you
// reply in place. What waits is a comment whose latest reply is an agent's, on
// a track you own; "Everyone's" shows those on everyone's tracks and on docs of
// no track too. Replying takes the comment off the list, and the next one opens.

export function Inbox({
  workspaceId,
  selected,
  onSelect,
}: {
  workspaceId: string;
  /** The open comment's ID, from the URL; the first item opens when it is not on the list. */
  selected: string | undefined;
  onSelect: (commentId: string) => void;
}) {
  const { projects, homes, parts, me, members, reload, failed } = useHomeData(workspaceId);
  const [everyone, setEveryone] = useEveryone();
  const groups = waitingGroups(homes, me, everyone);
  const items = groups?.flatMap((group) => group.items) ?? [];
  // Where the open item was, so that when it leaves the list the one after it opens.
  const place = useRef(0);
  const found = items.findIndex((item) => item.comment.id === selected);
  const at = found >= 0 ? found : Math.min(place.current, items.length - 1);
  const open = at >= 0 ? items[at] : undefined;
  useEffect(() => {
    if (at >= 0) place.current = at;
  }, [at]);
  const composer = useRef<HTMLTextAreaElement>(null);
  // Said once a reply is sent, while the next item opens.
  const [sent, setSent] = useState<string | null>(null);
  useEffect(() => {
    if (!sent) return;
    const timer = setTimeout(() => setSent(null), 4000);
    return () => clearTimeout(timer);
  }, [sent]);

  // J and K move through the list, R writes; not while typing.
  const move = (step: number) => {
    const next = items[Math.min(items.length - 1, Math.max(0, at + step))];
    if (next) onSelect(next.comment.id);
  };
  const latest = useRef({ move, write: () => composer.current?.focus() });
  latest.current = { move, write: () => composer.current?.focus() };
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      if (event.metaKey || event.ctrlKey || event.altKey || target?.closest("input, textarea, select, [contenteditable=true], [role=dialog]")) return;
      const key = event.key.toLowerCase();
      if (key === "j") latest.current.move(1);
      else if (key === "k") latest.current.move(-1);
      else if (key === "r") {
        event.preventDefault();
        latest.current.write();
      } else return;
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="absolute inset-0 grid grid-cols-1 min-[720px]:grid-cols-[minmax(320px,400px)_minmax(0,1fr)]">
      <section aria-label="Inbox items" className="flex min-h-0 min-w-0 flex-col border-r border-line">
        <header className="flex h-(--bar-h) shrink-0 items-center gap-2 border-b border-line pr-3 pl-4">
          <h1 className="text-section font-semibold tracking-[-0.006em]">Inbox</h1>
          {groups && (
            <span role="status" className="text-meta font-medium whitespace-nowrap text-ink-3 tabular-nums">
              {items.length} waiting
            </span>
          )}
          <div role="radiogroup" aria-label="Whose items" className="ml-auto flex shrink-0 gap-0.5 rounded-control bg-hover p-0.5">
            {([false, true] as const).map((value) => (
              <button
                key={String(value)}
                type="button"
                role="radio"
                aria-checked={everyone === value}
                onClick={() => setEveryone(value)}
                className="h-6 rounded-badge px-2.5 font-medium whitespace-nowrap text-ink-2 transition-colors hover:text-ink focus-visible:shadow-focus focus-visible:outline-none aria-checked:bg-sheet aria-checked:text-ink aria-checked:shadow-[0_0_0_1px_var(--line),var(--shadow-control-value)]"
              >
                {value ? "Everyone’s" : "Mine"}
              </button>
            ))}
          </div>
        </header>
        <div role="listbox" aria-label="Waiting on you" className="min-h-0 flex-1 overflow-y-auto">
          {failed && <p className="px-4 py-6 text-danger-ink">The inbox could not be loaded. Try again in a moment.</p>}
          {groups?.length === 0 && (
            <p className="px-4 py-6 text-ink-3">{everyone ? "Nothing waits on anyone." : "Nothing waits on you."}</p>
          )}
          {groups?.map((group) => (
            <ListGroup
              key={group.key}
              workspaceId={workspaceId}
              group={group}
              projects={projects}
              parts={parts}
              members={members}
              me={me}
              open={open}
              onSelect={onSelect}
            />
          ))}
        </div>
      </section>
      <section aria-label="Open item" className="relative hidden min-h-0 min-w-0 flex-col min-[720px]:flex">
        {open ? (
          <OpenItem
            key={open.comment.id}
            workspaceId={workspaceId}
            item={open}
            projects={projects}
            members={members}
            me={me}
            composer={composer}
            onReplied={(to) => {
              setSent(`Reply sent to the ${to === "You" ? "comment" : to}. It leaves your inbox.`);
              reload();
            }}
          />
        ) : (
          groups && <AllClear workspaceId={workspaceId} />
        )}
        <p role="status" className="pointer-events-none absolute inset-x-0 bottom-6 flex justify-center">
          {sent && <span className="animate-pop-in rounded-control bg-ink px-3 py-2 font-medium text-on-fill shadow-pop">{sent}</span>}
        </p>
      </section>
    </div>
  );
}

/** A track's sticky band and its items. */
function ListGroup({
  workspaceId,
  group,
  projects,
  parts,
  members,
  me,
  open,
  onSelect,
}: {
  workspaceId: string;
  group: WaitingGroup;
  projects: readonly ProjectBody[] | null;
  parts: ReadonlyMap<string, PartBody[]> | null;
  members: MemberBody[] | null;
  me: string | null;
  open: WaitingItem | undefined;
  onSelect: (commentId: string) => void;
}) {
  const track = group.track?.track;
  const part = track ? findPart(parts, group.projectId, track.part_id) : undefined;
  const project = projects?.find((candidate) => candidate.id === group.projectId);
  const owner = track && track.owner_user_id !== me ? memberName(members, track.owner_user_id) : null;
  const name = track?.name ?? `${project?.name ?? "Project"}, no track`;
  return (
    <div role="group" aria-label={name}>
      <div data-ib-band className="sticky top-0 z-[1] flex h-(--band-h) items-center gap-2 bg-band pr-3 pl-4 font-medium text-ink">
        {track ? <StateIcon state={track.state} className="mx-[3px]" /> : <ProjectMark name={project?.name ?? ""} className="size-(--icon-sm)" />}
        {track ? (
          <Link {...trackLink(workspaceId, track)} className="min-w-0 truncate rounded-badge hover:underline hover:decoration-line-strong hover:underline-offset-2">
            {name}
          </Link>
        ) : (
          <span className="min-w-0 truncate">{name}</span>
        )}
        <span className="mr-1 font-medium text-ink-3 tabular-nums">{group.items.length}</span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5">
          {owner && track && <Avatar name={owner} tintKey={track.owner_user_id ?? undefined} size="sm" title={`${owner}’s track`} />}
          {part && (
            <span className="inline-flex items-center gap-1.5 text-meta font-medium text-ink-3">
              <PartIcon name={part.name} kind={part.kind} tone={partTone(parts?.get(group.projectId), part.id)} size="sm" />
              {part.name}
            </span>
          )}
        </span>
      </div>
      {group.items.map((item) => {
        const who = whoOf(item.reply.author, members, me);
        const chosen = item === open;
        return (
          <div
            key={item.comment.id}
            role="option"
            aria-selected={chosen}
            tabIndex={chosen ? 0 : -1}
            onClick={() => onSelect(item.comment.id)}
            onKeyDown={(event: ReactKeyboardEvent) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onSelect(item.comment.id);
              }
            }}
            className={cn(
              "grid cursor-pointer grid-cols-[var(--avatar)_minmax(0,1fr)] gap-x-3 gap-y-0.5 border-t border-line px-4 py-2.5 transition-colors duration-(--t-fast) ease-out-quint [div[data-ib-band]+&]:border-t-0",
              "hover:bg-frame focus-visible:shadow-[inset_var(--focus-ring)] focus-visible:outline-none aria-selected:bg-selected",
            )}
          >
            <Avatar name={who.avatarName ?? who.name} agent={who.agent} tintKey={who.tintKey} className="row-span-2" />
            <span className="flex min-w-0 items-baseline gap-2">
              <span className="min-w-0 flex-1 truncate font-medium text-ink">{firstLine(item.reply.text)}</span>
              <span className="shrink-0 text-meta whitespace-nowrap text-ink-3 tabular-nums">{age(item.at)}</span>
            </span>
            <span className="min-w-0 truncate text-meta text-ink-3">
              <b className="font-medium text-ink-2">{who.name}</b>
              {item.doc ? ` · ${item.doc.title}` : ""}
            </span>
          </div>
        );
      })}
    </div>
  );
}

/** The open item: where it sits, who replied and what, your comment and the replies before, and the reply box. */
function OpenItem({
  workspaceId,
  item,
  projects,
  members,
  me,
  composer,
  onReplied,
}: {
  workspaceId: string;
  item: WaitingItem;
  projects: readonly ProjectBody[] | null;
  members: MemberBody[] | null;
  me: string | null;
  composer: React.RefObject<HTMLTextAreaElement | null>;
  /** Called with the name of whom the reply went to. */
  onReplied: (to: string) => void;
}) {
  const { comment, reply, doc } = item;
  const track = item.track?.track;
  const project = projects?.find((candidate) => candidate.id === item.projectId);
  const who = whoOf(reply.author, members, me);
  const author = whoOf(comment.author, members, me);
  const owner = track && track.owner_user_id !== me ? memberName(members, track.owner_user_id) : null;
  const docTitle = doc?.title ?? "the doc";
  const where = docLink(workspaceId, item.projectId, doc ?? { id: comment.doc_id, track_id: comment.track_id });
  const earlier = [comment, ...comment.replies.slice(0, -1)];
  const [text, setText] = useState("");
  const [state, setState] = useState<"idle" | "empty" | "sending" | "failed">("idle");

  async function send() {
    if (!text.trim()) {
      setState("empty");
      composer.current?.focus();
      return;
    }
    setState("sending");
    try {
      await replyToComment(workspaceId, item.projectId, comment.id, text.trim());
      setText("");
      setState("idle");
      onReplied(who.name);
    } catch {
      setState("failed");
    }
  }

  const pathLink = "inline-flex h-6 min-w-0 items-center gap-1.5 rounded-badge px-1.5 whitespace-nowrap text-ink-2 transition-colors hover:bg-hover hover:text-ink";
  return (
    <>
      <header className="flex h-(--bar-h) shrink-0 items-center gap-3 border-b border-line pr-3 pl-8">
        <nav aria-label="Where this sits" className="-ml-1.5 flex min-w-0 flex-1 items-center gap-0.5 font-medium">
          {track ? (
            <Link {...trackLink(workspaceId, track)} className={cn(pathLink, "shrink text-ink")}>
              <StateIcon state={track.state} />
              <span className="min-w-0 truncate">{track.name}</span>
            </Link>
          ) : (
            <Link to="/w/$workspaceId/projects/$projectId" params={{ workspaceId, projectId: item.projectId }} className={cn(pathLink, "shrink text-ink")}>
              <span className="min-w-0 truncate">{project?.name ?? "Project"}</span>
            </Link>
          )}
          <ChevronRight aria-hidden className="size-3 shrink-0 text-faint" />
          <Link {...where} search={{ comment: comment.id }} className={cn(pathLink, "shrink")}>
            <span className="min-w-0 truncate">{docTitle}</span>
          </Link>
        </nav>
        <Link {...where} search={{ comment: comment.id }} className={buttonClass({ ghost: true, sm: true })}>
          Open
        </Link>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="max-w-[calc(var(--main-max)+64px)] px-8 pt-7 pb-10">
          {owner && (
            <p className="mb-5 flex gap-2 rounded-control bg-frame px-3 py-2.5 text-ink-2">
              <User aria-hidden className="mt-px size-(--icon-sm) shrink-0" />
              <span>
                <b className="font-semibold text-ink">{owner}&rsquo;s track.</b> You read and comment; {owner} and agents on
                their key change it.
              </span>
            </p>
          )}
          <div className="flex items-center gap-2 text-ink-3">
            <Avatar name={who.avatarName ?? who.name} agent={who.agent} tintKey={who.tintKey} />
            <b className="font-semibold text-ink">{who.name}</b>
            <span>
              replied · <time dateTime={reply.created_at} title={new Date(reply.created_at).toLocaleString()}>{age(item.at)}</time>
            </span>
          </div>
          <h2
            className={cn(
              "mt-3 font-semibold break-words whitespace-pre-wrap text-ink [text-wrap:pretty]",
              reply.text.length > 140 ? "text-section leading-(--lh-prose)" : "text-title leading-[1.3] tracking-(--track-title)",
            )}
          >
            {reply.text.trim()}
          </h2>

          <div className="mt-6 overflow-hidden rounded-card border border-line">
            <div className="flex h-(--band-h) items-center gap-2 bg-band px-3 text-meta font-medium text-ink-2">
              <MessageSquare aria-hidden className="size-(--icon-sm)" strokeWidth={2.25} />
              {author.name === "You" ? "Your comment" : `${author.name}’s comment`} on {docTitle}
            </div>
            <div className="space-y-3 p-3">
              {comment.orphaned && <ElementSnapshot kind={doc?.kind === "html" ? "html" : "design"} snapshot={comment.snapshot ?? ""} />}
              {earlier.map((message) => {
                const writer = whoOf(message.author, members, me);
                return (
                  <div key={message.id} className="grid grid-cols-[var(--avatar)_minmax(0,1fr)] gap-x-2.5">
                    <Avatar name={writer.avatarName ?? writer.name} agent={writer.agent} tintKey={writer.tintKey} />
                    <div className="flex h-(--avatar) items-center gap-2">
                      <span className="font-semibold">{writer.name}</span>
                      <span className="text-meta text-ink-3">{age(Date.parse(message.created_at))}</span>
                    </div>
                    <p className="col-start-2 mt-0.5 leading-(--lh-prose) break-words whitespace-pre-wrap">{message.text}</p>
                  </div>
                );
              })}
            </div>
          </div>

          <div className="mt-6">
            <div
              data-invalid={state === "empty" || undefined}
              className="flex flex-col rounded-card border border-line-strong bg-sheet shadow-control transition-[border-color,box-shadow] duration-(--t-fast) ease-out-quint focus-within:border-brand focus-within:shadow-focus data-invalid:border-danger"
            >
              <textarea
                ref={composer}
                rows={3}
                value={text}
                onChange={(event) => {
                  setText(event.target.value);
                  if (state === "empty") setState("idle");
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && (isMac() ? event.metaKey : event.ctrlKey)) {
                    event.preventDefault();
                    void send();
                  }
                }}
                placeholder={`Reply on ${docTitle}`}
                aria-label={`Reply on ${docTitle}`}
                aria-describedby={state === "empty" || state === "failed" ? "inbox-reply-error" : undefined}
                className="min-h-[76px] resize-none border-0 bg-transparent px-3 pt-2.5 leading-(--lh-body) outline-none placeholder:text-ink-3"
              />
              <div className="flex items-center gap-2 py-2 pr-2 pl-3">
                <span className="flex flex-1 items-center gap-1 text-meta text-ink-3">
                  <Kbd>{isMac() ? "⌘" : "Ctrl"}</Kbd>
                  <Kbd>↵</Kbd>
                  to send
                </span>
                <button type="button" disabled={state === "sending"} onClick={() => void send()} className={buttonClass({ primary: true })}>
                  Send reply
                </button>
              </div>
            </div>
            {(state === "empty" || state === "failed") && (
              <p id="inbox-reply-error" className="mt-1.5 text-meta font-medium text-danger-ink">
                {state === "empty" ? "Write a reply before you send it." : "The reply could not be sent. Try again in a moment."}
              </p>
            )}
          </div>
          <p className="mt-6 flex flex-wrap gap-x-5 gap-y-1 text-meta text-ink-3">
            <span className="inline-flex items-center gap-1">
              <Kbd>J</Kbd>
              <Kbd>K</Kbd> next and previous
            </span>
            <span className="inline-flex items-center gap-1">
              <Kbd>R</Kbd> write
            </span>
            <span className="inline-flex items-center gap-1">
              <Kbd>{isMac() ? "⌘" : "Ctrl"}</Kbd>
              <Kbd>↵</Kbd> send
            </span>
          </p>
        </div>
      </div>
    </>
  );
}

/** The right pane when nothing waits. */
function AllClear({ workspaceId }: { workspaceId: string }) {
  return (
    <div className="flex max-w-[520px] flex-col items-start gap-2 px-8 py-12">
      <span className="mb-2 grid size-10 place-items-center rounded-card bg-frame shadow-[inset_0_0_0_1px_var(--line)]">
        <CircleCheck aria-hidden className="size-5 text-ink-2" strokeWidth={2} />
      </span>
      <h2 className="text-section font-semibold">You&rsquo;re all caught up</h2>
      <p className="mb-2 text-ink-2">When an agent replies to a comment on one of your tracks, it shows up here, and you reply in place.</p>
      <Link {...workspaceLink(workspaceId, "inbox")} className={buttonClass()}>
        Back to For you
      </Link>
    </div>
  );
}
