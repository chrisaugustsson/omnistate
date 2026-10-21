import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { createLiveClient, type LiveClient, type LiveEvent } from "./client";

const LiveContext = createContext<LiveClient | null>(null);

/** Gives the views of a workspace one live-event client, which opens its socket on the first subscription. */
export function LiveProvider({ workspaceId, children }: { workspaceId: string; children: ReactNode }) {
  const [client, setClient] = useState<LiveClient | null>(null);
  useEffect(() => {
    const created = createLiveClient({ workspaceId });
    setClient(created);
    return () => created.close();
  }, [workspaceId]);
  return <LiveContext.Provider value={client}>{children}</LiveContext.Provider>;
}

/** Calls `onEvent` for every live event of the project while the component is mounted. */
export function useProjectEvents(projectId: string, onEvent: (event: LiveEvent) => void) {
  const client = useContext(LiveContext);
  const latest = useRef(onEvent);
  useEffect(() => {
    latest.current = onEvent;
  });
  useEffect(
    () => client?.subscribe(projectId, (event) => latest.current(event)),
    [client, projectId],
  );
}
