import type { TreeNode } from "@/api/behaviour";
import { cn } from "@/lib/utils";

// The marks on a box and on a row: waiting questions, answers by default, a pointer, a node
// that waits for the next grill pass.

/** The marks of `node`; `count` replaces its own number of waiting questions, `titleOf` names pointer targets. */
export function Marks({
  node,
  waiting,
  count,
  countTitle,
  titleOf,
}: {
  node: TreeNode;
  waiting: ReadonlyMap<string, number>;
  count?: number;
  countTitle?: string;
  titleOf?: ReadonlyMap<string, string>;
}) {
  const open = count ?? waiting.get(node.id) ?? 0;
  const pointers = [
    node.same_as ? { glyph: "↪", label: "Same as", target: node.same_as.id } : null,
    node.handoff ? { glyph: "⇢", label: "Hands off to", target: node.handoff.id } : null,
  ];
  return (
    <>
      {node.needs_pass && (
        <i data-mark="pass" title="Needs another grill pass" className="text-xs font-semibold not-italic leading-none text-foreground">
          ↻
        </i>
      )}
      {node.default_answers > 0 && (
        <i
          data-mark="default"
          title={`${node.default_answers} ${node.default_answers === 1 ? "answer" : "answers"} given by default`}
          className="mx-[3px] size-[7px] shrink-0 rotate-45 border-[1.5px] border-muted-foreground"
        />
      )}
      {pointers.map((pointer) => {
        if (!pointer) return null;
        const title = titleOf?.get(pointer.target);
        return (
          <i
            key={pointer.glyph}
            data-mark="pointer"
            title={title ? `${pointer.label}: ${title}` : pointer.label}
            className="text-xs not-italic leading-none text-muted-foreground"
          >
            {pointer.glyph}
          </i>
        );
      })}
      {open > 0 && (
        <i
          data-mark="waiting"
          title={countTitle ?? (open === 1 ? "1 open question" : `${open} open questions`)}
          className="h-4 min-w-4 shrink-0 rounded-full bg-signal px-1 text-center text-[11px] font-medium not-italic leading-4 tabular-nums text-signal-foreground"
        >
          {open}
        </i>
      )}
    </>
  );
}

const swatch = "grid h-[11px] w-[18px] place-items-center rounded-[3px] border bg-background";

/** What the boxes and marks mean, top left of the graph. */
export function Legend() {
  const item = "inline-flex items-center gap-[5px] whitespace-nowrap";
  return (
    <div
      data-no-pan
      aria-label="Legend"
      className="absolute left-3 top-3 flex w-fit max-w-[calc(100%-24px)] flex-wrap items-center gap-x-3.5 gap-y-1 rounded-lg border bg-background px-2.5 py-1.5 text-[11.5px] text-muted-foreground"
    >
      <span className={item}>
        <i className={swatch} /> Done
      </span>
      <span className={item}>
        <i className={swatch}>
          <i className="size-[5px] rounded-full border-[1.5px] border-signal" />
        </i>{" "}
        Open
      </span>
      <span className={item}>
        <i className={cn(swatch, "border-dashed bg-transparent")} /> Out of scope
      </span>
      <span className={item}>
        <i className="h-4 min-w-4 rounded-full bg-signal px-1 text-center text-[11px] font-medium not-italic leading-4 text-signal-foreground">1</i> Open questions
      </span>
      <span className={item}>
        <i className="mx-[3px] size-[7px] rotate-45 border-[1.5px] border-muted-foreground" /> Answered by default
      </span>
      <span className={item}>
        <i className={cn(swatch, "border-dashed border-foreground")} /> <i className="text-xs font-semibold not-italic text-foreground">↻</i> Needs another grill pass
      </span>
      <span className={item}>
        <i className="text-xs not-italic">↪</i> Same as
      </span>
      <span className={item}>
        <i className="text-xs not-italic">⇢</i> Hands off to
      </span>
    </div>
  );
}
