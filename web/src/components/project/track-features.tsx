import { useCallback } from "react";
import { Link } from "@tanstack/react-router";
import { loadTrackFeatures } from "@/api/features";
import { Empty, List, rowClass } from "@/components/project/items";
import { useReloading } from "@/lib/reloading";
import { useProjectEvents } from "@/live/context";

type Params = { workspaceId: string; projectId: string };

/** The features that a track changes, each opening its feature page. Read-only: linking is done in the CLI. */
export function TrackFeatures({ workspaceId, projectId, trackId }: Params & { trackId: string }) {
  const load = useCallback(() => loadTrackFeatures(workspaceId, projectId, trackId), [workspaceId, projectId, trackId]);
  const { value: features, reload } = useReloading(load);
  useProjectEvents(projectId, reload);

  return (
    <section id="features" aria-labelledby="features-title" className="scroll-mt-6 space-y-3">
      <div className="flex items-baseline gap-2.5">
        <h2 id="features-title" className="text-[15px] font-semibold tracking-tight">
          Features
        </h2>
        <span className="text-sm text-muted-foreground tabular-nums">{features?.length ?? 0}</span>
      </div>
      {features &&
        (features.length === 0 ? (
          <Empty>This track is not linked to any feature</Empty>
        ) : (
          <List>
            {features.map((feature) => (
              <li key={feature.id}>
                <Link
                  to="/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId"
                  params={{ workspaceId, projectId, partId: feature.part_id, featureId: feature.id }}
                  className={rowClass}
                >
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium">{feature.name}</span>
                    {feature.description && (
                      <span className="mt-0.5 block truncate text-[13px] text-muted-foreground">
                        {feature.description.split(/\r?\n/, 1)[0]}
                      </span>
                    )}
                  </span>
                </Link>
              </li>
            ))}
          </List>
        ))}
    </section>
  );
}
