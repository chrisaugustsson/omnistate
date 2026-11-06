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
import type { Interaction, InteractionAction } from "@/design/renderer";

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

const TRIGGER_LABELS: Record<string, string> = {
  onClick: "On click",
  onHover: "While hovering",
  onPress: "While pressing",
  onDrag: "On drag",
  mouseEnter: "Mouse enter",
  mouseLeave: "Mouse leave",
  mouseDown: "Mouse down",
  mouseUp: "Mouse up",
};

/** The label of an interaction's trigger, such as `On click`, `Key K` or `After 2 s`. */
export function triggerLabel(trigger: Interaction["trigger"]): string {
  if (trigger.type === "afterDelay") return `After ${(trigger.ms ?? 0) / 1000} s`;
  if (trigger.type === "keyPress") {
    const key = trigger.key ?? "";
    return key.startsWith("Gamepad ") ? key : `Key ${key}`;
  }
  return TRIGGER_LABELS[trigger.type] ?? trigger.type;
}

const OPERATORS: Record<string, string> = {
  add: "+",
  subtract: "-",
  multiply: "*",
  divide: "/",
  equals: "==",
  notEqual: "!=",
  less: "<",
  lessOrEqual: "<=",
  greater: ">",
  greaterOrEqual: ">=",
  and: "&&",
  or: "||",
};

function literal(value: unknown): string {
  return typeof value === "string" ? JSON.stringify(value) : String(value);
}

/** An expression of a set variable or a condition, written as an infix formula. */
export function expressionText(expr: unknown, nested = false): string {
  if (typeof expr !== "object" || expr === null) return literal(expr);
  const node = expr as { value?: unknown; variable?: string; op?: string; args?: unknown[]; mode?: string };
  if ("variable" in node) return String(node.variable);
  if ("value" in node) return literal(node.value);
  if (node.op && Array.isArray(node.args)) {
    const args = node.args.map((arg) => expressionText(arg, true));
    const operator = OPERATORS[node.op];
    if (operator && args.length === 2) {
      const text = `${args[0]} ${operator} ${args[1]}`;
      return nested ? `(${text})` : text;
    }
    if (node.op === "not") return `!${args[0]}`;
    if (node.op === "negate") return `-${args[0]}`;
    return `${node.op}(${args.join(", ")})`;
  }
  return JSON.stringify(expr);
}

const POSITION_LABELS: Record<string, string> = {
  center: "center",
  topLeft: "top left",
  topCenter: "top center",
  topRight: "top right",
  bottomLeft: "bottom left",
  bottomCenter: "bottom center",
  bottomRight: "bottom right",
  manual: "manual",
};

const MAX_URL = 40;

/** What an action does and to which target, written as the inspect panel shows it. */
export function actionText(action: InteractionAction): string {
  const named = (verb: string, kind: string) =>
    action.destName ? `${verb} ${action.destName}` : `${verb.replace(/ (to|with)$/, "")} a deleted ${kind}`;
  switch (action.type) {
    case "navigate":
      return action.destName ? `Navigate to ${action.destName}` : "Navigate to a deleted board";
    case "back":
      return "Back";
    case "scrollTo": {
      const [x, y] = action.offset ?? [0, 0];
      return `${action.destName ? `Scroll to ${action.destName}` : "Scroll to a deleted layer"}${x || y ? ` (offset ${x}, ${y})` : ""}`;
    }
    case "openOverlay":
      return `${named("Open overlay", "board")} (${POSITION_LABELS[action.position ?? "center"] ?? action.position})`;
    case "swapOverlay":
      return action.destName ? `Swap overlay with ${action.destName}` : "Swap overlay with a deleted board";
    case "closeOverlay":
      return "Close overlay";
    case "openUrl": {
      const url = action.url ?? "";
      return `Open ${url.length > MAX_URL ? `${url.slice(0, MAX_URL)}…` : url}`;
    }
    case "changeTo":
      return action.destName ? `Change to ${action.destName}` : "Change to a deleted component";
    case "setVariable":
      return `Set ${action.variable} to ${action.expr !== undefined ? expressionText(action.expr) : literal(action.value)}`;
    case "setVariableMode":
      return `Set mode of ${action.collection} to ${action.mode}`;
    case "conditional": {
      const count = action.blocks?.length ?? 0;
      return `Conditional · ${count} ${count === 1 ? "branch" : "branches"}`;
    }
    default:
      return action.type;
  }
}

/** The lines under a conditional action: one per branch, `if <condition>: <actions>` or `else: <actions>`. */
export function branchLines(action: InteractionAction): string[] {
  return (action.blocks ?? []).map((block) => {
    const actions = block.actions
      .map((inner) => {
        const text = actionText(inner);
        return text.charAt(0).toLowerCase() + text.slice(1);
      })
      .join("; ");
    return block.condition === undefined ? `else: ${actions}` : `if ${expressionText(block.condition)}: ${actions}`;
  });
}

const TRANSITION_VERBS: Record<string, string> = {
  dissolve: "Dissolve",
  smartAnimate: "Smart animate",
  moveIn: "Move in from",
  moveOut: "Move out to",
  push: "Push from",
  slideIn: "Slide in from",
  slideOut: "Slide out to",
};

function easingText(easing: Interaction["easing"]): string {
  if (typeof easing === "string") return easing.replace(/[A-Z]/g, (letter) => ` ${letter.toLowerCase()}`);
  if (easing.type === "customBezier") return `cubic-bezier(${[easing.x1, easing.y1, easing.x2, easing.y2].join(", ")})`;
  return "spring";
}

function hasAction(actions: InteractionAction[], test: (action: InteractionAction) => boolean): boolean {
  return actions.some((action) => test(action) || hasAction(action.blocks?.flatMap((block) => block.actions) ?? [], test));
}

/** The second line of an interaction: its transition, duration, easing, delay and flags. */
export function transitionText(interaction: Interaction): string {
  const { transition } = interaction;
  const verb = TRANSITION_VERBS[transition.type];
  const parts =
    transition.type === "instant" || !verb
      ? ["Instant"]
      : [
          transition.direction ? `${verb} ${transition.direction}` : verb,
          `${interaction.durationMs} ms`,
          easingText(interaction.easing),
        ];
  if (interaction.delayMs) parts.push(`delay ${interaction.delayMs} ms`);
  if (hasAction(interaction.actions, (action) => action.type === "navigate" && action.preserveScroll === true)) {
    parts.push("keep scroll");
  }
  if (hasAction(interaction.actions, (action) => action.type === "openUrl" && action.newTab !== false)) {
    parts.push("new tab");
  }
  return parts.join(" · ");
}

/** The first target of the interaction that is not in the doc. */
function goneTarget(actions: InteractionAction[]): string | null {
  for (const action of actions) {
    if (action.dest && !action.destName) return action.dest;
    const inner = goneTarget(action.blocks?.flatMap((block) => block.actions) ?? []);
    if (inner) return inner;
  }
  return null;
}

/**
 * The selected layer's interactions, read-only: a row each with the trigger,
 * the actions and the transition. An interaction whose target is gone is drawn
 * in the warning style.
 */
export function InteractionsSection({ interactions }: { interactions: Interaction[] }) {
  return (
    <section aria-label="Interactions" className="space-y-2">
      <h3 className="text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
        Interactions ({interactions.length})
      </h3>
      <ul className="space-y-2">
        {interactions.map((interaction, index) => {
          const gone = interaction.dangling ? goneTarget(interaction.actions) : null;
          return (
            <li
              key={index}
              data-dangling={interaction.dangling ? "" : undefined}
              className={cn(
                "space-y-1.5 rounded-lg px-2.5 py-2 text-[12px]",
                interaction.dangling
                  ? "bg-orange-50 text-orange-700 ring-1 ring-orange-600/60 ring-inset dark:bg-orange-400/10 dark:text-orange-300"
                  : "bg-muted text-foreground",
              )}
            >
              {interaction.actions.map((action, at) => (
                <div key={at} className="space-y-0.5">
                  <p className="flex min-w-0 flex-wrap items-center gap-1.5">
                    {at === 0 && (
                      <span className="rounded-[4px] bg-black/10 px-1.5 py-px text-[11px] font-medium dark:bg-white/10">
                        {triggerLabel(interaction.trigger)}
                      </span>
                    )}
                    <span aria-hidden className="text-muted-foreground">
                      →
                    </span>
                    <span className="min-w-0 break-words">{actionText(action)}</span>
                  </p>
                  {branchLines(action).map((line) => (
                    <p key={line} className="text-[11px] text-muted-foreground">
                      {line}
                    </p>
                  ))}
                </div>
              ))}
              <p className="text-[11px] text-muted-foreground">{transitionText(interaction)}</p>
              {gone && <p className="text-[11px]">Target {gone} is gone. The player ignores this interaction.</p>}
            </li>
          );
        })}
      </ul>
      <p className="text-[11px] text-muted-foreground">Read-only. Agents write interactions with omni design edit.</p>
    </section>
  );
}

/** How a board or frame scrolls its content, and for a frame where it sits when the board scrolls. */
export function ScrollingRow({ overflow, position }: { overflow: string; position?: string | null }) {
  return (
    <div className="flex min-w-0 items-baseline justify-between gap-2 text-[12px]">
      <span className="text-muted-foreground">Scrolling</span>
      <span className="font-mono text-[11px]">
        overflow {overflow}
        {position ? ` · position ${position}` : ""}
      </span>
    </div>
  );
}
