import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Maximize2 } from "lucide-react";
import type { HomeDoc } from "@/api/generated";
import { Avatar } from "@/components/shell/avatar";
import { DocPage, type DocSearch } from "@/components/viewer/doc-page";
import { timeAgo } from "@/lib/format";
import { cn } from "@/lib/utils";
import { TAB_PILLARS, useTrack, type DocTab } from "./track-data";
import { EmptyTab, buttonClass } from "./ui";

// The Spec, Design and How tabs of a track: the track's doc of that pillar in
// the doc viewer, filling the room under the tabs. A tab with several docs
// names them above the viewer to pick from; a tab with none says which agent
// writes it.

const EMPTY: Record<DocTab, { title: string; body: string }> = {
  spec: {
    title: "No spec yet",
    body: "The grill agent writes the track's spec here while it grills the idea with you (omni-grill).",
  },
  design: {
    title: "No design yet",
    body: "The design agent draws the track's screens on design boards here: layout options first, then the finished design.",
  },
  how: {
    title: "No how yet",
    body: "The how agent writes how the track is built here, its operations and tables, once its behaviour and design are settled (omni-how).",
  },
};

/** The docs of a tab in the order of its pillars, then the design system first, then by title. */
function inOrder(docs: HomeDoc[], pillars: readonly string[]): HomeDoc[] {
  return [...docs].sort(
    (a, b) =>
      pillars.indexOf(a.pillar) - pillars.indexOf(b.pillar) ||
      Number(b.kind === "design-system") - Number(a.kind === "design-system") ||
      a.title.toLowerCase().localeCompare(b.title.toLowerCase()),
  );
}

function writer(doc: HomeDoc): string {
  return doc.last_writer_role ? `${doc.last_writer_role.charAt(0).toUpperCase()}${doc.last_writer_role.slice(1)} agent` : "";
}

export function TrackDocTab({ tab, search }: { tab: DocTab; search: DocSearch }) {
  const { workspaceId, projectId, trackId, home, docs: all } = useTrack();
  const docs = inOrder(all[tab], TAB_PILLARS[tab]);
  const [chosen, setChosen] = useState<string | null>(null);
  const doc = docs.find((one) => one.id === chosen) ?? docs[0];

  if (!home) return <div aria-hidden className="h-40 animate-pulse rounded-card bg-frame" />;
  if (!doc) return <EmptyTab title={EMPTY[tab].title}>{EMPTY[tab].body}</EmptyTab>;
  const who = writer(doc);
  return (
    <div className="flex flex-1 flex-col">
      <div className="flex min-h-7 flex-wrap items-center gap-x-2 gap-y-1 text-meta text-ink-3">
        {docs.length > 1 && (
          <div role="group" aria-label="Docs" className="mr-2 flex flex-wrap gap-1">
            {docs.map((one) => (
              <button
                key={one.id}
                type="button"
                aria-pressed={one.id === doc.id}
                onClick={() => setChosen(one.id)}
                className={cn(buttonClass({ variant: "ghost", size: "sm" }), "text-body")}
              >
                {one.title}
              </button>
            ))}
          </div>
        )}
        {who && <Avatar name={who} agent size="sm" />}
        <span>
          {who && <b className="font-medium text-ink-2">{who}</b>}
          {doc.updated_at && (
            <span title={new Date(doc.updated_at).toLocaleString()}>
              {who ? " · " : ""}updated {timeAgo(doc.updated_at)}
            </span>
          )}
        </span>
        <Link
          to="/w/$workspaceId/projects/$projectId/tracks/$trackId/docs/$docId"
          params={{ workspaceId, projectId, trackId, docId: doc.id }}
          className={cn(buttonClass({ variant: "ghost", size: "sm" }), "ml-auto")}
        >
          <Maximize2 aria-hidden strokeWidth={2.25} />
          Full view
        </Link>
      </div>
      <div className="relative mt-3 min-h-[560px] flex-1 overflow-hidden rounded-card border border-line">
        <div className="absolute inset-0 overflow-auto">
          <DocPage key={doc.id} workspaceId={workspaceId} projectId={projectId} docId={doc.id} search={search} />
        </div>
      </div>
    </div>
  );
}
