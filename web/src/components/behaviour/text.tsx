import { Fragment, type ReactNode } from "react";

// A node's text: paragraphs, "- " lists, **bold** and `code`. A link keeps only its label;
// everything else is plain text, never HTML.

const INLINE = /\*\*(.+?)\*\*|`([^`]+)`|\[([^\]]+)\]\([^)]*\)/g;

function inline(line: string): ReactNode[] {
  const parts: ReactNode[] = [];
  let at = 0;
  for (const match of line.matchAll(INLINE)) {
    if (match.index > at) parts.push(line.slice(at, match.index));
    if (match[1] !== undefined) parts.push(<strong key={match.index}>{match[1]}</strong>);
    else if (match[2] !== undefined) parts.push(<code key={match.index} className="rounded bg-muted px-1 font-mono text-[0.92em]">{match[2]}</code>);
    else parts.push(match[3]);
    at = match.index + match[0].length;
  }
  if (at < line.length) parts.push(line.slice(at));
  return parts;
}

/** `source` as paragraphs and lists. */
export function Text({ source, className }: { source: string; className?: string }) {
  const blocks: ReactNode[] = [];
  let items: string[] = [];
  const flush = () => {
    if (!items.length) return;
    const list = items;
    items = [];
    blocks.push(
      <ul key={`l${blocks.length}`} className="list-disc space-y-0.5 pl-5">
        {list.map((item, at) => (
          <li key={at}>{inline(item)}</li>
        ))}
      </ul>,
    );
  };
  for (const line of source.split("\n")) {
    const item = /^\s*- (.*)$/.exec(line);
    if (item) items.push(item[1]);
    else {
      flush();
      if (line.trim()) blocks.push(<p key={`p${blocks.length}`}>{inline(line)}</p>);
    }
  }
  flush();
  return (
    <div className={className ?? "space-y-2"}>
      {blocks.map((block, at) => (
        <Fragment key={at}>{block}</Fragment>
      ))}
    </div>
  );
}
