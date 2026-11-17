import type { CommentBody, HomeDoc, HomeResponse, MemberBody, ReplyBody, TrackSummary } from "@/api/generated";
import { personName } from "@/lib/identity";

// What Home shows, as pure functions over every project's home (build plan,
// "Data" 6): the comments that wait on the reader, grouped by track, and the
// reader's tracks. A comment waits when its latest reply is an agent's
// (`home.waiting.comments`); it waits on the reader when it is on a track the
// reader owns, as the Inbox count in the top bar has it (waitingOnMe).

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
  track: TrackSummary | undefined;
};

/** The waiting comments of one track, or of one project's docs that are on no track. */
export type WaitingGroup = {
  key: string;
  projectId: string;
  track: TrackSummary | undefined;
  /** The newest reply of the group, in ms. */
  at: number;
  items: WaitingItem[];
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

/** A member's name by user ID; null while unknown. */
export function memberName(members: readonly MemberBody[] | null, userId: string | null | undefined): string | null {
  const member = userId ? members?.find((candidate) => candidate.user_id === userId) : undefined;
  return member ? personName(member.email) : null;
}

/**
 * The comments that wait, grouped by track, the group with the newest reply
 * first and the newest reply first in each group. Only those on tracks `me`
 * owns, unless `everyone`; then also those on other people's tracks and on
 * docs that are on no track. Null while the homes or the reader are unknown.
 */
export function waitingGroups(
  homes: ReadonlyMap<string, HomeResponse> | null,
  me: string | null,
  everyone = false,
): WaitingGroup[] | null {
  if (!homes || (!me && !everyone)) return null;
  const groups = new Map<string, WaitingGroup>();
  for (const [projectId, home] of homes) {
    const tracks = new Map((home.tracks ?? []).map((summary) => [summary.track.id, summary]));
    const docs = new Map(home.docs.map((doc) => [doc.id, doc]));
    for (const comment of home.waiting.comments) {
      const reply = comment.replies.at(-1);
      if (!reply) continue;
      const track = comment.track_id ? tracks.get(comment.track_id) : undefined;
      if (!everyone && (!track || track.track.owner_user_id !== me)) continue;
      const key = track ? track.track.id : `project:${projectId}`;
      const item: WaitingItem = { comment, reply, at: Date.parse(reply.created_at) || 0, projectId, doc: docs.get(comment.doc_id), track };
      const group = groups.get(key) ?? { key, projectId, track, at: 0, items: [] };
      group.items.push(item);
      group.at = Math.max(group.at, item.at);
      groups.set(key, group);
    }
  }
  for (const group of groups.values()) group.items.sort((a, b) => b.at - a.at);
  return [...groups.values()].sort((a, b) => b.at - a.at);
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

/** "Good morning" before noon, "Good afternoon" until six, then "Good evening" (and before five). */
export function greeting(now: Date): string {
  const hour = now.getHours();
  if (hour >= 5 && hour < 12) return "Good morning";
  if (hour >= 12 && hour < 18) return "Good afternoon";
  return "Good evening";
}

/** "1 comment" / "3 comments". */
export function plural(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`;
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
 * off to Wallets on iot-service, building" (the first track split off it); empty
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
  return `split off to ${child.track.name}${where}, ${child.track.state}`;
}

/** The first line of a text, for a one-line row. */
export function firstLine(text: string): string {
  return text.trim().split("\n", 1)[0] ?? "";
}
