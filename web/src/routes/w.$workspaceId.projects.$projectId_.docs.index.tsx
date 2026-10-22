import { useState } from "react";
import { createFileRoute } from "@tanstack/react-router";
import { DocRow, Empty, List, MODULES } from "@/components/project/items";
import { FilterTabs, SectionPage } from "@/components/project/section-page";

// The project's docs: the PRD, architecture, DB and UI docs, grouped by
// module, with a tab for written docs and one for designs.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/docs/")({
  component: Docs,
});

type Tab = "all" | "html" | "design";

function Docs() {
  const { workspaceId, projectId } = Route.useParams();
  const [tab, setTab] = useState<Tab>("all");
  const params = { workspaceId, projectId };

  return (
    <SectionPage
      workspaceId={workspaceId}
      projectId={projectId}
      title="Docs"
      description={() => "The plans agents write: the PRD, the architecture, the database and the UI."}
    >
      {(home) => {
        const docs = home.docs.filter((doc) => tab === "all" || doc.kind === tab);
        return (
          <>
            <FilterTabs
              label="Kind of doc"
              value={tab}
              onChange={setTab}
              tabs={[
                { value: "all", label: "All", count: home.docs.length },
                { value: "html", label: "Docs", count: home.docs.filter((doc) => doc.kind === "html").length },
                { value: "design", label: "Designs", count: home.docs.filter((doc) => doc.kind === "design").length },
              ]}
            />
            {docs.length === 0 && <Empty>No docs here yet. Agents add them through change sets.</Empty>}
            {MODULES.map(({ module, title, blurb }) => {
              const inModule = docs.filter((doc) => doc.module === module);
              if (!inModule.length) return null;
              const id = `docs-${module}`;
              return (
                <section key={module} aria-labelledby={id} className="space-y-2">
                  <div className="flex items-baseline gap-2">
                    <h2 id={id} className="text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
                      {title}
                    </h2>
                    <span className="text-xs text-muted-foreground/70">{blurb}</span>
                  </div>
                  <List>
                    {inModule.map((doc) => (
                      <DocRow key={doc.id} doc={doc} params={params} />
                    ))}
                  </List>
                </section>
              );
            })}
          </>
        );
      }}
    </SectionPage>
  );
}
