import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

type Variant = "outline" | "signal" | "amber" | "green" | "violet" | "blue" | "red";

/** The colour of each status the API sends, for change sets, decisions, comments and claims. */
const VARIANTS: Record<string, Variant> = {
  open: "outline",
  draft: "outline",
  in_review: "amber",
  proposed: "signal",
  addressed: "blue",
  approved: "green",
  merged: "violet",
  resolved: "green",
  rejected: "red",
  closed: "outline",
  sealed: "violet",
};

const DOTS: Record<Variant, string> = {
  outline: "bg-muted-foreground/60",
  signal: "bg-signal",
  amber: "bg-amber-500",
  green: "bg-emerald-500",
  violet: "bg-violet-500",
  blue: "bg-sky-500",
  red: "bg-red-500",
};

/** A status as words: `in_review` is "in review". */
export function statusWords(status: string): string {
  return status.replace(/_/g, " ");
}

/** A status as a coloured pill with a dot, its text the status in words. */
export function StatusBadge({ status, className }: { status: string; className?: string }) {
  const variant = VARIANTS[status] ?? "outline";
  return (
    <Badge variant={variant} className={cn("capitalize", className)}>
      <span aria-hidden className={cn("size-1.5 rounded-full", DOTS[variant])} />
      {statusWords(status)}
    </Badge>
  );
}
