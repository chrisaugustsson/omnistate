import { useCallback, useState } from "react";
import { createFileRoute } from "@tanstack/react-router";
import { createKey, loadKeys, revokeKey } from "@/api/keys";
import { PageHeader } from "@/components/page-header";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { timeAgo } from "@/lib/format";
import { useReloading } from "@/lib/reloading";

// The workspace's API keys: what agents sign in with. Only the logged-in
// reviewer manages them. A new key's secret is shown once, when it is created.
export const Route = createFileRoute("/w/$workspaceId/api-keys")({
  component: ApiKeys,
});

function ApiKeys() {
  const { workspaceId } = Route.useParams();
  const { value: keys, failed, reload } = useReloading(useCallback(() => loadKeys(workspaceId), [workspaceId]));
  const [secret, setSecret] = useState<string | null>(null);
  const [error, setError] = useState(false);
  const [busy, setBusy] = useState(false);

  async function act(run: () => Promise<void>) {
    setBusy(true);
    setError(false);
    try {
      await run();
      reload();
    } catch {
      setError(true);
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="mx-auto max-w-3xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader
        title="API keys"
        description="Agents sign in with a key. Create one and give it to the agent's harness."
        actions={
          <Button
            disabled={busy}
            onClick={() =>
              void act(async () => {
                setSecret((await createKey(workspaceId)).secret);
              })
            }
          >
            Create key
          </Button>
        }
      />
      {(failed || error) && (
        <Alert variant="destructive" className="mt-6">
          <AlertDescription>That did not work. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {secret && (
        <Alert className="mt-6">
          <AlertDescription>
            <p>Copy the key now. It is not shown again.</p>
            <code className="mt-2 block break-all font-mono text-xs text-foreground">{secret}</code>
          </AlertDescription>
        </Alert>
      )}
      <ul className="mt-8 divide-y overflow-hidden rounded-xl border bg-card">
        {keys?.length === 0 && <li className="px-4 py-6 text-sm text-muted-foreground">No keys yet.</li>}
        {keys?.map((key) => (
          <li key={key.id} className="flex items-center gap-3 px-4 py-3 text-sm">
            <span className="font-mono text-xs">{key.prefix}</span>
            <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
              {key.revoked_at
                ? `Revoked ${timeAgo(key.revoked_at)}`
                : key.last_used_at
                  ? `Used ${timeAgo(key.last_used_at)}`
                  : "Not used yet"}
            </span>
            {!key.revoked_at && (
              <Button variant="ghost" size="sm" disabled={busy} onClick={() => void act(() => revokeKey(workspaceId, key.id))}>
                Revoke
              </Button>
            )}
          </li>
        ))}
      </ul>
    </main>
  );
}
