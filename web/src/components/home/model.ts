import type { CommentBody, HomeDoc, HomeResponse, MemberBody, ReplyBody, TrackSummary } from "@/api/generated";
import { personName } from "@/lib/identity";

// What the workspace's pages show, as pure functions over every project's
// home (build plan, "Data" 6): the comments that wait on the reader, and the
// reader's tracks. A comment waits when its latest reply is an agent's
// (`home.waiting.comments`); it waits on the reader when it is on a track the
// reader owns, as the Inbox count in the header has it (waitingOnMe).

/** Who wrote something, as Home shows them: a person by name, an agent as "<role> agent". */
export type Who = {
  name: string;
  agent: boolean;
  tintKey?: string;
  /** The name the avatar draws its initial from, when not `name` ("You" draws the reader's own). */
  avatarName?: string;
};

/** A comment that waits: the comment, the reply that makes it wait, the doc it is on and its track. */
export type WaitingItem = {
  comment: CommentBody;
  reply: ReplyBody;
  /** When the reply was written, in ms. */
  at: number;
  projectId: string;
  doc: HomeDoc | undefined;
  track: TrackSummary;
};

/** "Writer agent" for the agent role "writer". */
function agentName(role: unknown): string {
  const name = typeof role === "string" && role ? role : "";
  return name ? `${name.charAt(0).toUpperCase()}${name.slice(1)} agent` : "Agent";
}

/** Who an author is: "You", a member by name (decision U8), or an agent by role. */
export function whoOf(author: Record<string, unknown>, members: readonly MemberBody[] | null, me: string | null): Who {
  if (author.kind !== "human") return { name: agentName(author.role), agent: true };
  const userId = typeof author.user_id === "string" ? author.user_id : undefined;
  const member = members?.find((candidate) => candidate.user_id === userId);
  if (userId && userId === me) return { name: "You", agent: false, tintKey: userId, avatarName: member ? personName(member.email) : undefined };
  return { name: member ? personName(member.email) : "Someone", agent: false, tintKey: userId };
}

/**
 * The comments that wait on `me`: those on tracks `me` owns whose latest reply
 * is an agent's, in every project, the newest reply first. Null while the
 * homes or the reader are unknown.
 */
export function waitingItems(homes: ReadonlyMap<string, HomeResponse> | null, me: string | null): WaitingItem[] | null {
  if (!homes || !me) return null;
  const items: WaitingItem[] = [];
  for (const [projectId, home] of homes) {
    const tracks = new Map((home.tracks ?? []).map((summary) => [summary.track.id, summary]));
    const docs = new Map(home.docs.map((doc) => [doc.id, doc]));
    for (const comment of home.waiting.comments) {
      const reply = comment.replies.at(-1);
      const track = comment.track_id ? tracks.get(comment.track_id) : undefined;
      if (!reply || !track || track.track.owner_user_id !== me) continue;
      items.push({ comment, reply, at: Date.parse(reply.created_at) || 0, projectId, doc: docs.get(comment.doc_id), track });
    }
  }
  return items.sort((a, b) => b.at - a.at);
}

/** The order of the track states, as the Tracks list groups them. */
export const STATE_ORDER = ["planning", "building", "done", "dropped"] as const;

/** When a track last moved, in ms: its last activity, or its creation while it has none. */
export function activityOf(summary: TrackSummary): number {
  return Date.parse(summary.last_activity ?? summary.track.created_at) || 0;
}

/** The tracks `me` owns in every project, the most recently active first; null while unknown. */
export function myTracks(homes: ReadonlyMap<string, HomeResponse> | null, me: string | null): TrackSummary[] | null {
  if (!homes || !me) return null;
  const mine = [...homes.values()].flatMap((home) => (home.tracks ?? []).filter(({ track }) => track.owner_user_id === me));
  return mine.sort((a, b) => activityOf(b) - activityOf(a));
}

/** How long ago, short, as rows show it: "now", "12 min", "3 h", "Yesterday", "4 d", or the date. */
export function age(at: number, now = Date.now()): string {
  const minutes = Math.round(Math.max(0, now - at) / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h`;
  const days = Math.round(hours / 24);
  if (days === 1) return "Yesterday";
  if (days < 7) return `${days} d`;
  return new Date(at).toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

/**
 * How a track came apart, in a few words: "split off from Checkout", or "split
 * off to Wallets on iot-service" (the first track split off it); empty
 * when it is neither.
 */
export function splitText(summary: TrackSummary, home: HomeResponse | undefined): string {
  const tracks = home?.tracks ?? [];
  const parent = summary.track.split_from_track_id
    ? tracks.find(({ track }) => track.id === summary.track.split_from_track_id)
    : undefined;
  if (parent) return `split off from ${parent.track.name}`;
  const child = tracks.find(({ track }) => track.split_from_track_id === summary.track.id);
  if (!child) return "";
  const where = child.part_name && child.part_name !== summary.part_name ? ` on ${child.part_name}` : "";
  return `split off to ${child.track.name}${where}`;
}
