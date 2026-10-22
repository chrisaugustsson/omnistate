import type { ReactNode } from "react";
import {
  Circle,
  Component,
  Frame,
  Group,
  Image as ImageIcon,
  PenTool,
  RectangleHorizontal,
  Shapes,
  Square,
  Type,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";

// Small pieces of the design viewer's panes: the icon of a layer's type, a
// colour swatch, a token chip and the property table of the inspect panel.

const LAYER_ICONS: Record<string, LucideIcon> = {
  board: Frame,
  frame: Square,
  group: Group,
  text: Type,
  image: ImageIcon,
  path: PenTool,
  vector: PenTool,
  svg: PenTool,
  line: PenTool,
  ellipse: Circle,
  rect: RectangleHorizontal,
  instance: Component,
  component: Component,
};

/** The icon of a layer's type. */
export function LayerIcon({ type, className }: { type: string; className?: string }) {
  const Icon = LAYER_ICONS[type] ?? Shapes;
  return <Icon aria-hidden className={cn("size-3.5 shrink-0", className)} />;
}

const COLOUR = /#(?:[0-9a-f]{8}|[0-9a-f]{6}|[0-9a-f]{3,4})\b|\b(?:rgba?|hsla?|oklch|oklab|lab|lch)\([^)]*\)/i;

/** The first colour a value names (a hex code or a CSS colour function), if any. */
export function colourIn(text: string): string | null {
  return COLOUR.exec(text)?.[0] ?? null;
}

/** A small square of a colour, on a checkerboard so that transparency shows. */
export function Swatch({ colour }: { colour: string }) {
  return (
    <span
      aria-hidden
      className="relative inline-block size-3 shrink-0 overflow-hidden rounded-[3px] ring-1 ring-foreground/15 ring-inset"
      style={{
        backgroundImage:
          "linear-gradient(45deg,#e5e5e5 25%,transparent 25%),linear-gradient(-45deg,#e5e5e5 25%,transparent 25%),linear-gradient(45deg,transparent 75%,#e5e5e5 75%),linear-gradient(-45deg,transparent 75%,#e5e5e5 75%)",
        backgroundSize: "6px 6px",
        backgroundPosition: "0 0,0 3px,3px -3px,-3px 0",
      }}
    >
      <span className="absolute inset-0" style={{ background: colour }} />
    </span>
  );
}

/** A value in mono type, with a swatch in front when it names a colour. */
export function MonoValue({ text, className }: { text: string; className?: string }) {
  const colour = colourIn(text);
  return (
    <span className={cn("inline-flex max-w-full min-w-0 items-start gap-1.5", className)}>
      {colour && (
        <span className="flex h-[1lh] items-center">
          <Swatch colour={colour} />
        </span>
      )}
      <span className="min-w-0 font-mono break-all">{text}</span>
    </span>
  );
}

/** A design token's name as a small chip. */
export function TokenChip({ name }: { name: string }) {
  return (
    <span className="inline-flex max-w-full items-center rounded-[5px] bg-violet-50 px-1.5 py-px font-mono text-[10.5px] leading-4 break-all text-violet-800 ring-1 ring-violet-600/15 ring-inset dark:bg-violet-400/10 dark:text-violet-300">
      {name}
    </span>
  );
}

/**
 * A group of the inspect panel: a table named by its label, with the property
 * names in the first column and their values in the second.
 */
export function PropertyTable({ label, rows }: { label: string; rows: { name: string; cells: ReactNode }[] }) {
  return (
    <table aria-label={label} className="w-full table-fixed border-separate border-spacing-0 text-left text-[12px]">
      <caption className="pb-1.5 text-left text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
        {label}
      </caption>
      <tbody>
        {rows.map((row) => (
          <tr key={row.name} className="align-top">
            <th
              scope="row"
              className="w-[38%] truncate py-1 pr-2 text-left font-normal text-muted-foreground"
              title={row.name}
            >
              {row.name}
            </th>
            <td className="py-1 text-foreground">{row.cells}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
