import { Fragment } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronRight } from "lucide-react";
import { sectionLink, type Section } from "@/components/project/items";
import { useHome, useProjectName } from "@/lib/use-home";
import { cn } from "@/lib/utils";

const SECTION_LABEL: Record<Section, string> = { docs: "Docs", changesets: "Change sets", threads: "Threads" };

/**
 * Where an item lives: the project, the feature when the item has one, and the
 * section of its kind, each a link to its page.
 */
export function Trail({
  workspaceId,
  projectId,
  featureId,
  section,
  className,
}: {
  workspaceId: string;
  projectId: string;
  /** The item's feature; `null` or absent for a project-level item. */
  featureId?: string | null;
  section: Section;
  className?: string;
}) {
  const projectName = useProjectName(workspaceId, projectId);
  const { home } = useHome(workspaceId, projectId);
  const featureName = home?.features?.find((summary) => summary.feature.id === featureId)?.feature.name;
  const crumbs = [
    {
      key: "project",
      label: projectName ?? "Project",
      link: { to: "/w/$workspaceId/projects/$projectId", params: { workspaceId, projectId } },
    },
    ...(featureId
      ? [
          {
            key: "feature",
            label: featureName ?? "Feature",
            link: {
              to: "/w/$workspaceId/projects/$projectId/features/$featureId",
              params: { workspaceId, projectId, featureId },
            },
          },
        ]
      : []),
    { key: "section", label: SECTION_LABEL[section], link: sectionLink(section, { workspaceId, projectId }, featureId) },
  ];
  return (
    <nav aria-label="Breadcrumb" className={className}>
      <ol className="flex min-w-0 flex-wrap items-center gap-x-1 gap-y-0.5 text-xs font-medium text-muted-foreground">
        {crumbs.map(({ key, label, link }, index) => (
          <Fragment key={key}>
            {index > 0 && <ChevronRight aria-hidden className="size-3 shrink-0 text-muted-foreground/50" />}
            <li className="min-w-0">
              <Link
                {...link}
                activeOptions={{ exact: true }}
                className={cn(
                  "block max-w-48 truncate rounded transition-colors hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none",
                  index === crumbs.length - 1 && "text-foreground",
                )}
              >
                {label}
              </Link>
            </li>
          </Fragment>
        ))}
      </ol>
    </nav>
  );
}
