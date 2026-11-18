import type { HomeDoc } from "@/api/generated";
import { Avatar } from "@/components/shell/avatar";
import { timeAgo } from "@/lib/format";

// Who last wrote a doc and when, as the slim bar over a doc says it:
// "Spec agent · updated 9 h ago" with the agent's avatar.

/** The agent that last wrote `doc` ("Spec agent"), or "" when no agent did. */
export function docWriter(doc: Pick<HomeDoc, "last_writer_role">): string {
  return doc.last_writer_role ? `${doc.last_writer_role.charAt(0).toUpperCase()}${doc.last_writer_role.slice(1)} agent` : "";
}

export function DocByline({ doc }: { doc: Pick<HomeDoc, "last_writer_role" | "updated_at"> }) {
  const who = docWriter(doc);
  return (
    <span className="flex min-w-0 items-center gap-2">
      {who && <Avatar name={who} agent size="sm" />}
      <span className="min-w-0 truncate">
        {who && <b className="font-medium text-ink-2">{who}</b>}
        {doc.updated_at && (
          <span title={new Date(doc.updated_at).toLocaleString()}>
            {who ? " · " : ""}updated {timeAgo(doc.updated_at)}
          </span>
        )}
      </span>
    </span>
  );
}
