import { useCallback, useState } from "react";
import { createFileRoute } from "@tanstack/react-router";
import { Check, Copy, KeyRound, Plus } from "lucide-react";
import type { ApiKeyBody } from "@/api/generated";
import { createKey, loadKeys, revokeKey } from "@/api/keys";
import { btn, btnDanger, btnGhostSm, btnPrimary, ColumnList, EmptyRow, rowClass, RowLead, ScopePage, Sep } from "@/components/project/rows";
import { Dialog } from "@/components/shell/dialog";
import { PageHead } from "@/components/shell/page";
import { timeAgo } from "@/lib/format";
import { useReloading } from "@/lib/reloading";
import { cn } from "@/lib/utils";

// Your API keys: what agents sign in with. A key is yours, not the
// workspace's: it reaches every workspace you are a member of, so the page
// shows the same keys from any workspace. Only the logged-in reviewer manages
// them. A new key's secret is shown once, in a dialog, when it is created;
// revoking one asks first, since the agents using it are refused from then on.
export const Route = createFileRoute("/w/$workspaceId/_home/api-keys")({
  component: ApiKeys,
});

function used(key: ApiKeyBody): string {
  if (key.revoked_at) return `Revoked ${timeAgo(key.revoked_at)}`;
  return key.last_used_at ? `Used ${timeAgo(key.last_used_at)}` : "Not used yet";
}

/** The one-time secret of a new key, with a Copy button. */
function SecretDialog({ secret, onClose }: { secret: string; onClose: () => void }) {
  const [copied, setCopied] = useState(false);
  return (
    <Dialog
      title="Copy your new key"
      description="It is not shown again. Give it to the agent's harness; you can revoke it here at any time."
      onClose={onClose}
      onSubmit={onClose}
      footer={
        <button type="submit" className={btnPrimary}>
          Done
        </button>
      }
    >
      <div className="flex items-center gap-2 rounded-control border border-line bg-frame py-1.5 pr-1.5 pl-3">
        <code aria-label="New key" className="min-w-0 flex-1 font-mono text-[12px] break-all text-ink">
          {secret}
        </code>
        <button
          type="button"
          data-autofocus
          className={btn}
          onClick={() => {
            void navigator.clipboard?.writeText(secret).then(() => setCopied(true));
          }}
        >
          {copied ? <Check aria-hidden /> : <Copy aria-hidden />}
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
    </Dialog>
  );
}

function ApiKeys() {
  const { value: keys, failed, reload } = useReloading(useCallback(() => loadKeys(), []));
  const [secret, setSecret] = useState<string | null>(null);
  const [revoking, setRevoking] = useState<ApiKeyBody | null>(null);
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

  const live = keys?.filter((key) => !key.revoked_at) ?? [];
  const revoked = keys?.filter((key) => key.revoked_at) ?? [];

  return (
    <ScopePage>
      <PageHead
        title="API keys"
        actions={
          <button
            type="button"
            className={btnPrimary}
            disabled={busy}
            onClick={() =>
              void act(async () => {
                setSecret((await createKey()).secret);
              })
            }
          >
            <Plus aria-hidden strokeWidth={2.25} />
            Create key
          </button>
        }
      />
      <p className="mt-1.5 max-w-(--main-max) text-em text-ink-2">
        Agents sign in with a key. A key is yours and reaches every workspace you are a member of. Create one and give it to the agent's harness.
      </p>
      {(failed || error) && (
        <p role="alert" className="mt-4 text-danger-ink">
          That did not work. Try again in a moment.
        </p>
      )}
      {keys && (
        <section aria-label="Keys" className="mt-8">
          <ColumnList label="Keys">
            {keys.length === 0 && <EmptyRow icon={<KeyRound aria-hidden className="size-(--icon) text-faint" />}>No keys yet. Create one for each agent harness.</EmptyRow>}
            {[...live, ...revoked].map((key) => (
              <li key={key.id} className={rowClass()}>
                <RowLead>
                  <KeyRound aria-hidden className={cn("size-(--icon-sm)", key.revoked_at ? "text-faint" : "text-ink-2")} />
                </RowLead>
                <span className={cn("font-mono text-[12px]", key.revoked_at ? "text-ink-3 line-through decoration-ink-3" : "text-ink")}>{key.prefix}…</span>
                <span className="min-w-0 flex-1 truncate text-meta text-ink-3">
                  Created {timeAgo(key.created_at)}
                  <Sep />
                  {used(key)}
                </span>
                {!key.revoked_at && (
                  <button type="button" className={btnGhostSm} disabled={busy} onClick={() => setRevoking(key)}>
                    Revoke
                  </button>
                )}
              </li>
            ))}
          </ColumnList>
        </section>
      )}
      {secret && <SecretDialog secret={secret} onClose={() => setSecret(null)} />}
      {revoking && (
        <Dialog
          title={`Revoke ${revoking.prefix}…?`}
          description="Agents using this key are refused from then on. A revoked key stays listed, struck through."
          onClose={() => setRevoking(null)}
          onSubmit={() => {
            const key = revoking;
            setRevoking(null);
            void act(() => revokeKey(key.id));
          }}
          footer={
            <>
              <button type="button" className={btn} onClick={() => setRevoking(null)}>
                Cancel
              </button>
              <button type="submit" data-autofocus className={btnDanger}>
                Revoke key
              </button>
            </>
          }
        />
      )}
    </ScopePage>
  );
}
