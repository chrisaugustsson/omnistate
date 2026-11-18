import { useCallback } from "react";
import type { MemberBody } from "@/api/generated";
import { loadMembers, loadMyUserId } from "@/api/members";
import { Avatar } from "@/components/shell/avatar";
import { personName } from "@/lib/identity";
import { useReloading } from "@/lib/reloading";
import { useProjectEvents } from "@/live/context";
import { cn } from "@/lib/utils";

// Who people are on the feature and track pages: the workspace's members by
// the name their email gives (decision U8), the reader, and the authors of
// decisions and posts (a person, or "<role> agent").

export type People = {
  members: MemberBody[];
  /** The reader's user ID; null for an agent's key. */
  me: string | null;
  /** The reader's membership. */
  mine: MemberBody | undefined;
  admin: boolean;
};

/** The workspace's members and the reader, loaded again on every event of the project; null while they load. */
export function usePeople(workspaceId: string, projectId: string): People | null {
  const load = useCallback(async () => {
    const [members, me] = await Promise.all([loadMembers(workspaceId), loadMyUserId(workspaceId)]);
    const mine = members.find((member) => member.user_id === me);
    return { members, me, mine, admin: mine?.role === "admin" };
  }, [workspaceId]);
  const { value, reload } = useReloading(load, `people:${workspaceId}`);
  useProjectEvents(projectId, reload);
  return value;
}

/** A member's name as the pages show it ("Christopher"); "Someone" for a user who is not (or no longer) a member. */
export function nameOf(people: People | null, userId: string | null | undefined): string {
  if (!userId) return "Nobody";
  const member = people?.members.find((one) => one.user_id === userId);
  return member ? personName(member.email) : "Someone";
}

/** Who wrote something (a decision, a post): a member's name, or "<role> agent". */
export function authorOf(people: People | null, author: Record<string, unknown>): { name: string; agent: boolean; key?: string } {
  if (author.kind === "human") {
    const userId = typeof author.user_id === "string" ? author.user_id : null;
    return { name: userId ? nameOf(people, userId) : "Someone", agent: false, key: userId ?? undefined };
  }
  const role = typeof author.role === "string" && author.role ? author.role : "an";
  const name = `${role} agent`;
  return { name: name.charAt(0).toUpperCase() + name.slice(1), agent: true };
}

/** A person's avatar and name, with a muted "you" when it is the reader. */
export function Person({ people, userId, className }: { people: People | null; userId: string | null | undefined; className?: string }) {
  const name = nameOf(people, userId);
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-2", className)}>
      <Avatar name={name} tintKey={userId ?? undefined} />
      <span className="truncate font-medium text-ink">{name}</span>
      {userId && userId === people?.me && <span className="font-normal text-ink-3">you</span>}
    </span>
  );
}
