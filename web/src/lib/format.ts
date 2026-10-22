/** How long ago an ISO time was, in a few words: "just now", "5 min ago", "3 h ago", "2 d ago", or the date. */
export function timeAgo(iso: string, now = Date.now()): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const seconds = Math.max(0, Math.round((now - then) / 1000));
  if (seconds < 45) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  if (days < 7) return `${days} d ago`;
  return new Date(iso).toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

/** Who wrote something: "You" for the human, otherwise the agent's role. */
export function authorName(author: Record<string, unknown>): string {
  if (author.kind === "human") return "You";
  return typeof author.role === "string" && author.role ? author.role : "agent";
}
