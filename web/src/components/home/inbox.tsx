import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowUpRight, ChevronLeft, ChevronRight, CircleHelp } from "lucide-react";
import type { HomeDoc, MemberBody, QuestionBody, TrackSummary } from "@/api/generated";
import { loadOpenQuestions } from "@/api/behaviour";
import { loadHeadHtml, replyToComment } from "@/api/docs";
import { btn, btnPrimary } from "@/components/project/rows";
import { Avatar } from "@/components/shell/avatar";
import { Page } from "@/components/shell/page";
import { SECTION_LABELS, docLink, trackLink, workspaceLink, type TrackSection } from "@/components/shell/scope";
import { useIsPhone } from "@/components/shell/shell-context";
import { inFlight } from "@/components/shell/state-icon";
import { TAB_PILLARS } from "@/components/track/track-data";
import { ElementSnapshot } from "@/components/viewer/comments";
import { isMac } from "@/components/palette/palette";
import { useProjectsEvents } from "@/live/context";
import { useReloading } from "@/lib/reloading";
import { cn } from "@/lib/utils";
import { age, myTracks, waitingItems, whoOf, type WaitingItem } from "./model";
import { useHomeData } from "./use-home-data";

// The workspace's Inbox (navigation F, tasks/ui-redesign/nav5/option-f.html,
// screen 1): what waits on you as a list, newest first, with the open item
// beside it. A comment waits when an agent replied last on a track you own;
// you answer it in place, or resolve it, and it leaves the list. A track of
// yours with behaviour questions an agent asked is a row too, which opens the
// questions and leads to the track's Behaviour. The open row is in the URL
// (`?comment=` or `?questions=<track>`). On the phone the list is the page,
// and a row opens its item full screen.

/** Which row is open, as the URL says it. */
export type InboxSearch = { comment?: string; questions?: string };

/** Reads the open row from the URL's search. */
export function validateInboxSearch(search: Record<string, unknown>): InboxSearch {
  const text = (value: unknown) => (typeof value === "string" && value ? value : undefined);
  const comment = text(search.comment);
  if (comment) return { comment };
  const questions = text(search.questions);
  return questions ? { questions } : {};
}

type Entry =
  | { kind: "comment"; key: string; at: number; item: WaitingItem }
  | { kind: "questions"; key: string; at: number; track: TrackSummary; questions: QuestionBody[] };

const searchOf = (entry: Entry | undefined): InboxSearch =>
  !entry ? {} : entry.kind === "comment" ? { comment: entry.item.comment.id } : { questions: entry.track.track.id };

const keyOf = (search: InboxSearch) => (search.comment ? `c:${search.comment}` : search.questions ? `q:${search.questions}` : undefined);

/**
 * The open behaviour questions an agent asked on each of `tracks` in flight,
 * by track ID (a person's remark waits on no one); read again on any
 * behaviour event of their projects.
 */
function useQuestions(workspaceId: string, tracks: readonly TrackSummary[] | null) {
  const live = useMemo(() => (tracks ?? []).filter(({ track }) => inFlight(track.state)), [tracks]);
  const key = live.map(({ track }) => `${track.project_id}/${track.id}`).join(",");
  const load = useCallback(async () => {
    const pairs = key ? key.split(",").map((pair) => pair.split("/") as [string, string]) : [];
    const lists = await Promise.all(
      pairs.map(async ([projectId, trackId]) => {
        const questions = await loadOpenQuestions(workspaceId, projectId, trackId).catch(() => []);
        return [trackId, questions.filter((question) => question.asked_by.kind !== "human")] as const;
      }),
    );
    return new Map(lists);
  }, [workspaceId, key]);
  const { value, reload } = useReloading(load);
  const projectIds = useMemo(() => [...new Set(live.map(({ track }) => track.project_id))], [live]);
  useProjectsEvents(projectIds, (event) => {
    if (event.event.startsWith("behaviour.")) reload();
  });
  return tracks ? (value ?? null) : null;
}

/** The track section a doc shows in: Spec for spec and research, Design for UI, How for architecture and tables. */
function sectionOf(doc: HomeDoc | undefined): TrackSection {
  const pillar = doc?.pillar ?? "spec";
  for (const tab of ["spec", "design", "how"] as const) if ((TAB_PILLARS[tab] as readonly string[]).includes(pillar)) return tab;
  return "spec";
}

/**
 * Where "Open in track" goes: the track's section with the comment open, when
 * the section opens on this doc (its first, as the section orders them);
 * otherwise the doc itself under the track, with the comment open.
 */
function openInTrack(workspaceId: string, item: WaitingItem, docs: readonly HomeDoc[]) {
  const section = sectionOf(item.doc);
  const pillars = TAB_PILLARS[section as keyof typeof TAB_PILLARS] as readonly string[];
  const first = docs
    .filter((doc) => doc.track_id === item.track.track.id && pillars.includes(doc.pillar))
    .sort(
      (a, b) =>
        pillars.indexOf(a.pillar) - pillars.indexOf(b.pillar) ||
        Number(b.kind === "design-system") - Number(a.kind === "design-system") ||
        a.title.toLowerCase().localeCompare(b.title.toLowerCase()),
    )[0];
  const search = { comment: item.comment.id };
  if (!item.doc || first?.id === item.doc.id) return { ...trackLink(workspaceId, item.projectId, item.track.track.id, section), search };
  return { ...docLink(workspaceId, item.projectId, item.doc), search };
}

/** The text of the doc's element a comment is on, for an HTML doc: what the comment quotes. */
function useQuote(workspaceId: string, item: WaitingItem): string | null {
  const { projectId, doc, comment } = item;
  const html = doc?.kind === "html" && !comment.orphaned && /^[\w-]+$/.test(comment.node);
  const load = useCallback(async () => {
    if (!html || !doc) return null;
    const source = await loadHeadHtml(workspaceId, projectId, doc.id);
    const element = new DOMParser().parseFromString(source, "text/html").querySelector(`[data-omni-id="${comment.node}"]`);
    const text = (element?.textContent ?? "").replace(/\s+/g, " ").trim();
    return text.length > 280 ? `${text.slice(0, 280).trimEnd()}…` : text || null;
  }, [html, workspaceId, projectId, doc, comment.node]);
  return useReloading(load).value ?? null;
}

export function Inbox({
  workspaceId,
  search,
  onSelect,
}: {
  workspaceId: string;
  search: InboxSearch;
  /** Opens a row (or, with nothing, the list); `replace` for moves that should not be a step back. */
  onSelect: (search: InboxSearch, replace?: boolean) => void;
}) {
  const phone = useIsPhone();
  const { homes, me, members, reload, failed } = useHomeData(workspaceId);
  const items = waitingItems(homes, me);
  const mine = useMemo(() => myTracks(homes, me), [homes, me]);
  const questions = useQuestions(workspaceId, mine);
  const entries: Entry[] | null = useMemo(() => {
    if (!items || !mine || !questions) return null;
    const asked: Entry[] = mine.flatMap((track) => {
      const open = questions.get(track.track.id) ?? [];
      if (open.length === 0) return [];
      const at = Math.max(...open.map((question) => Date.parse(question.created_at) || 0));
      return [{ kind: "questions" as const, key: `q:${track.track.id}`, at, track, questions: open }];
    });
    const comments: Entry[] = items.map((item) => ({ kind: "comment" as const, key: `c:${item.comment.id}`, at: item.at, item }));
    return [...comments, ...asked].sort((a, b) => b.at - a.at);
  }, [items, mine, questions]);

  // Where the open row was, so that when it leaves the list the one after it
  // opens. With no row in the URL, the newest opens (on a desktop).
  const place = useRef(0);
  const list = entries ?? [];
  const wanted = keyOf(search);
  const found = list.findIndex((entry) => entry.key === wanted);
  const at = found >= 0 ? found : phone ? -1 : wanted ? Math.min(place.current, list.length - 1) : list.length > 0 ? 0 : -1;
  const open = at >= 0 ? list[at] : undefined;
  useEffect(() => {
    if (at >= 0) place.current = at;
  }, [at]);
  const composer = useRef<HTMLTextAreaElement>(null);
  const [sent, setSent] = useState<string | null>(null);
  useEffect(() => {
    if (!sent) return;
    const timer = setTimeout(() => setSent(null), 4000);
    return () => clearTimeout(timer);
  }, [sent]);

  // J and K move through the list, R writes; not while typing.
  const latest = useRef<{ move: (step: number) => void; write: () => void }>({ move: () => {}, write: () => {} });
  latest.current = {
    move: (step) => {
      const next = list[Math.min(list.length - 1, Math.max(0, at + step))];
      if (next) onSelect(searchOf(next), true);
    },
    write: () => composer.current?.focus(),
  };
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
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  if (failed) {
    return (
      <Page>
        <h1 className="sr-only">Inbox</h1>
        <p className="text-danger-ink">The inbox could not be loaded. Try again in a moment.</p>
      </Page>
    );
  }
  if (!entries) return <h1 className="sr-only">Inbox</h1>;
  if (entries.length === 0) {
    return (
      <Page>
        <h1 className="sr-only">Inbox</h1>
        <p className="text-ink-3 [&_a]:font-medium [&_a]:text-ink-2 [&_a:hover]:text-ink [&_a:hover]:underline [&_a:hover]:decoration-line-strong [&_a:hover]:underline-offset-2">
          Nothing waits on you. Your tracks are on <Link {...workspaceLink(workspaceId, "my-tracks")}>My tracks</Link>.
        </p>
      </Page>
    );
  }

  const done = (said: string) => {
    setSent(said);
    reload();
    if (phone) onSelect({}, true);
  };
  const item = open && (
    <OpenItem key={open.key} workspaceId={workspaceId} entry={open} docs={open.kind === "comment" ? (homes?.get(open.item.projectId)?.docs ?? []) : []} members={members} me={me} composer={composer} onDone={done} />
  );
  const toast = (
    <p role="status" className="pointer-events-none absolute inset-x-0 bottom-6 z-10 flex justify-center px-4">
      {sent && <span className="animate-pop-in rounded-control bg-ink px-3 py-2 font-medium text-on-fill shadow-pop">{sent}</span>}
    </p>
  );

  if (phone) {
    return (
      <div className="relative min-h-full">
        <h1 className="sr-only">Inbox</h1>
        {open ? (
          <section aria-label="Open item">
            <div className="sticky top-0 z-[1] flex h-11 items-center border-b border-line bg-sheet px-2">
              <Link
                to="/w/$workspaceId"
                params={{ workspaceId }}
                search={{}}
                className="inline-flex h-8 items-center gap-0.5 rounded-control pr-2 pl-1 font-medium text-ink-2 active:bg-hover focus-visible:shadow-focus focus-visible:outline-none"
              >
                <ChevronLeft aria-hidden className="size-4" strokeWidth={2.25} />
                Inbox
              </Link>
            </div>
            <div className="px-4 pt-4 pb-8">{item}</div>
          </section>
        ) : (
          <div className="px-4 pt-2 pb-6">
            <Rows workspaceId={workspaceId} entries={entries} open={undefined} members={members} me={me} />
          </div>
        )}
        {toast}
      </div>
    );
  }

  return (
    <div className="absolute inset-0 flex">
      <h1 className="sr-only">Inbox</h1>
      <div className="w-[340px] shrink-0 overflow-y-auto border-r border-line px-4 pt-3 pb-6 max-[1100px]:w-[300px]">
        <Rows workspaceId={workspaceId} entries={entries} open={open} members={members} me={me} />
      </div>
      <section aria-label="Open item" className="relative min-w-0 flex-1 overflow-y-auto">
        <div className="max-w-[760px] px-7 pt-5 pb-12">{item}</div>
        {toast}
      </section>
    </div>
  );
}

/** The list: one row per waiting comment, and one per track with behaviour questions. */
function Rows({
  workspaceId,
  entries,
  open,
  members,
  me,
}: {
  workspaceId: string;
  entries: Entry[];
  open: Entry | undefined;
  members: MemberBody[] | null;
  me: string | null;
}) {
  return (
    <ul aria-label="Waiting on you" className="flex flex-col">
      {entries.map((entry) => {
        const chosen = entry === open;
        let lead: ReactNode;
        let title: string;
        let track: TrackSummary;
        if (entry.kind === "comment") {
          const who = whoOf(entry.item.reply.author, members, me);
          lead = <Avatar name={who.avatarName ?? who.name} agent={who.agent} tintKey={who.tintKey} size="md" />;
          title = `${who.name} replied on ${entry.item.doc?.title ?? SECTION_LABELS[sectionOf(entry.item.doc)]}`;
          track = entry.item.track;
        } else {
          lead = (
            <span className="grid size-(--mark-md) shrink-0 place-items-center rounded-control border border-line bg-frame text-ink-2">
              <CircleHelp aria-hidden className="size-(--icon-sm)" strokeWidth={2.25} />
            </span>
          );
          title = `${entry.questions.length} behaviour ${entry.questions.length === 1 ? "question" : "questions"}`;
          track = entry.track;
        }
        return (
          <li key={entry.key} className="border-b border-line has-aria-[current=page]:border-transparent [&:has(+li_[aria-current=page])]:border-transparent">
            <Link
              to="/w/$workspaceId"
              params={{ workspaceId }}
              search={searchOf(entry)}
              aria-current={chosen ? "page" : undefined}
              activeOptions={{ exact: true, includeSearch: true }}
              activeProps={{}}
              inactiveProps={{ "aria-current": chosen ? "page" : undefined }}
              className={cn(
                "-mx-2 flex items-start gap-3 rounded-control px-2 py-2.5 text-ink transition-colors duration-(--t-fast) ease-out-quint",
                "hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none aria-[current=page]:bg-selected",
              )}
            >
              {lead}
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="truncate font-medium">{title}</span>
                <span className="truncate text-meta text-ink-3">
                  {track.track.name} · {age(entry.at)}
                </span>
              </span>
            </Link>
          </li>
        );
      })}
    </ul>
  );
}

const pathLink = "min-w-0 truncate rounded-badge text-ink-2 transition-colors hover:text-ink hover:underline hover:decoration-line-strong hover:underline-offset-2";

/** Where the open item sits: its track, then the section. */
function ItemPath({ workspaceId, track, section, link }: { workspaceId: string; track: TrackSummary; section: TrackSection; link: ReturnType<typeof trackLink> }) {
  return (
    <nav aria-label="Where this sits" className="flex min-w-0 items-center gap-1 text-meta font-medium">
      <Link {...trackLink(workspaceId, track.track.project_id, track.track.id, "spec")} className={pathLink}>
        {track.track.name}
      </Link>
      <ChevronRight aria-hidden className="size-3 shrink-0 text-faint" />
      <Link {...link} className={cn(pathLink, "shrink-0")}>
        {SECTION_LABELS[section]}
      </Link>
    </nav>
  );
}

/** The open item: a comment's thread with its reply box, or a track's behaviour questions. */
function OpenItem({
  workspaceId,
  entry,
  docs,
  members,
  me,
  composer,
  onDone,
}: {
  workspaceId: string;
  entry: Entry;
  docs: readonly HomeDoc[];
  members: MemberBody[] | null;
  me: string | null;
  composer: React.RefObject<HTMLTextAreaElement | null>;
  /** Called with what to say once the comment was answered or resolved. */
  onDone: (said: string) => void;
}) {
  if (entry.kind === "questions") {
    const { track } = entry;
    const behaviour = trackLink(workspaceId, track.track.project_id, track.track.id, "behaviour");
    return (
      <article aria-label={`Behaviour questions on ${track.track.name}`}>
        <ItemPath workspaceId={workspaceId} track={track} section="behaviour" link={behaviour} />
        <h2 className="mt-2 text-section font-semibold tracking-[-0.006em]">
          {entry.questions.length} behaviour {entry.questions.length === 1 ? "question" : "questions"}
        </h2>
        <p className="mt-1 text-ink-3">An agent asked; the answers shape the track&rsquo;s behaviour tree.</p>
        <ol className="mt-4 flex flex-col border-b border-line">
          {entry.questions.map((question) => (
            <li key={question.id} className="flex flex-col gap-0.5 border-t border-line py-2.5">
              <span className="leading-(--lh-prose) break-words">{question.question}</span>
              <span className="text-meta text-ink-3">
                {question.node_path} · {age(Date.parse(question.created_at))}
              </span>
            </li>
          ))}
        </ol>
        <Link {...behaviour} className={cn(btnPrimary, "mt-5")}>
          Answer on Behaviour
        </Link>
      </article>
    );
  }
  return <CommentItem workspaceId={workspaceId} item={entry.item} docs={docs} members={members} me={me} composer={composer} onDone={onDone} />;
}

/** A waiting comment's thread: the quoted text, the comment and its replies, and the box to reply or resolve. */
function CommentItem({
  workspaceId,
  item,
  docs,
  members,
  me,
  composer,
  onDone,
}: {
  workspaceId: string;
  item: WaitingItem;
  docs: readonly HomeDoc[];
  members: MemberBody[] | null;
  me: string | null;
  composer: React.RefObject<HTMLTextAreaElement | null>;
  onDone: (said: string) => void;
}) {
  const { comment, doc, track } = item;
  const section = sectionOf(doc);
  const quote = useQuote(workspaceId, item);
  const docTitle = doc?.title ?? SECTION_LABELS[section];
  const messages = [comment, ...comment.replies];
  const [text, setText] = useState("");
  const [state, setState] = useState<"idle" | "empty" | "sending" | "failed">("idle");

  // Resolving answers the comment with a last reply (yours, or "Resolved."), so no agent's reply waits on you.
  async function send(resolve: boolean) {
    const said = text.trim() || (resolve ? "Resolved." : "");
    if (!said) {
      setState("empty");
      composer.current?.focus();
      return;
    }
    setState("sending");
    try {
      await replyToComment(workspaceId, item.projectId, comment.id, said);
      setText("");
      setState("idle");
      onDone(resolve ? "Resolved. It leaves your inbox." : "Reply sent. It leaves your inbox.");
    } catch {
      setState("failed");
    }
  }

  return (
    <article aria-label={`Comment on ${docTitle}`}>
      <ItemPath workspaceId={workspaceId} track={track} section={section} link={trackLink(workspaceId, item.projectId, track.track.id, section)} />
      <h2 className="mt-2 text-section font-semibold tracking-[-0.006em]">{docTitle}</h2>
      {comment.orphaned ? (
        <div className="mt-3">
          <ElementSnapshot kind={doc?.kind === "html" ? "html" : "design"} snapshot={comment.snapshot ?? ""} />
        </div>
      ) : (
        quote && (
          <blockquote className="mt-3 border-l-2 border-line-strong py-0.5 pl-3 leading-(--lh-prose) break-words text-ink-2">
            &ldquo;{quote}&rdquo;
          </blockquote>
        )
      )}
      <ol aria-label="Thread" className="mt-4 flex flex-col gap-4">
        {messages.map((message) => {
          const writer = whoOf(message.author, members, me);
          return (
            <li key={message.id} className="grid grid-cols-[var(--mark-md)_minmax(0,1fr)] gap-x-2.5">
              <Avatar name={writer.avatarName ?? writer.name} agent={writer.agent} tintKey={writer.tintKey} size="md" />
              <div className="min-w-0">
                <div className="flex h-(--mark-md) items-center gap-2">
                  <span className="font-semibold">{writer.name}</span>
                  <time dateTime={message.created_at} title={new Date(message.created_at).toLocaleString()} className="text-meta text-ink-3">
                    {age(Date.parse(message.created_at))}
                  </time>
                </div>
                <p className="leading-(--lh-prose) break-words whitespace-pre-wrap">{message.text}</p>
              </div>
            </li>
          );
        })}
      </ol>

      <div className="mt-5">
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
                void send(false);
              }
            }}
            placeholder="Reply…"
            aria-label={`Reply on ${docTitle}`}
            aria-describedby={state === "empty" || state === "failed" ? "inbox-reply-error" : undefined}
            className="min-h-[76px] resize-none border-0 bg-transparent px-3 pt-2.5 leading-(--lh-body) outline-none placeholder:text-ink-3"
          />
          <div className="flex items-center justify-end gap-2 py-2 pr-2 pl-3">
            <button type="button" disabled={state === "sending"} onClick={() => void send(true)} className={btn} title="Close the comment with a last reply">
              Resolve
            </button>
            <button type="button" disabled={state === "sending"} onClick={() => void send(false)} className={btnPrimary}>
              Reply
            </button>
          </div>
        </div>
        {(state === "empty" || state === "failed") && (
          <p id="inbox-reply-error" className="mt-1.5 text-meta font-medium text-danger-ink">
            {state === "empty" ? "Write a reply before you send it." : "The reply could not be sent. Try again in a moment."}
          </p>
        )}
      </div>
      <Link {...openInTrack(workspaceId, item, docs)} className="mt-4 inline-flex items-center gap-1 font-medium text-ink-2 hover:text-ink hover:underline hover:decoration-line-strong hover:underline-offset-2">
        Open in track
        <ArrowUpRight aria-hidden className="size-(--icon-sm)" strokeWidth={2.25} />
      </Link>
    </article>
  );
}
