import { cn } from "@/lib/utils";

/** A hue from 0 to 359 that is always the same for the same name. */
function hueOf(name: string): number {
  let hash = 0;
  for (const char of name) hash = (hash * 31 + char.charCodeAt(0)) % 360;
  return hash;
}

/** Up to two letters for a role: `explorer:accessibility` is "EA", `committee-ux` is "CU", "You" is "Y". */
function initials(name: string): string {
  const parts = name.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  return (parts.length > 1 ? parts[0][0] + parts[1][0] : name.slice(0, 1)).toUpperCase();
}

/** A round avatar for an agent role or a person, coloured by the name. */
export function AgentAvatar({ name, className }: { name: string; className?: string }) {
  const hue = hueOf(name);
  return (
    <span
      aria-hidden
      className={cn(
        "inline-flex size-5 shrink-0 items-center justify-center rounded-full font-mono text-[9px] font-semibold tracking-tight",
        className,
      )}
      style={{ background: `oklch(0.93 0.05 ${hue})`, color: `oklch(0.4 0.12 ${hue})` }}
    >
      {initials(name)}
    </span>
  );
}

/** An agent role (or a person) as an avatar and its name in mono type. */
export function AgentChip({ name, className }: { name: string; className?: string }) {
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1.5 text-xs", className)}>
      <AgentAvatar name={name} />
      <span className="truncate font-mono text-[11px] text-foreground/80">{name}</span>
    </span>
  );
}
