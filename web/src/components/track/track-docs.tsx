import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Maximize2 } from "lucide-react";
import type { HomeDoc } from "@/api/generated";
import { DocByline } from "@/components/viewer/byline";
import { DocPage, type DocSearch } from "@/components/viewer/doc-page";
import { cn } from "@/lib/utils";
import { TAB_PILLARS, useTrack, type DocTab } from "./track-data";
import { EmptyTab, buttonClass } from "./ui";

// The Spec, Design and How tabs of a track: the track's doc of that pillar,
// read as page text under one slim bar (Spec, How) or as a canvas filling the
// room under the tabs (Design). A tab with several docs names them in the bar
// to pick from; a tab with none says which agent writes it.

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

export function TrackDocTab({ tab, search }: { tab: DocTab; search: DocSearch }) {
  const { workspaceId, projectId, trackId, home, docs: all } = useTrack();
  const docs = inOrder(all[tab], TAB_PILLARS[tab]);
  const [chosen, setChosen] = useState<string | null>(null);
  const doc = docs.find((one) => one.id === chosen) ?? docs[0];

  if (!home) return <div aria-hidden className="h-40 animate-pulse rounded-card bg-frame" />;
  if (!doc) return <EmptyTab title={EMPTY[tab].title}>{EMPTY[tab].body}</EmptyTab>;
  const meta = (
    <>
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
      <DocByline doc={doc} />
    </>
  );
  const fullView = (
    <Link
      to="/w/$workspaceId/projects/$projectId/tracks/$trackId/docs/$docId"
      params={{ workspaceId, projectId, trackId, docId: doc.id }}
      className={buttonClass({ variant: "ghost", size: "sm" })}
    >
      <Maximize2 aria-hidden strokeWidth={2.25} />
      Full view
    </Link>
  );
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <DocPage key={doc.id} workspaceId={workspaceId} projectId={projectId} docId={doc.id} search={search} chrome={{ meta, actions: fullView, embedded: true }} />
    </div>
  );
}
