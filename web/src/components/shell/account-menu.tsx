import { useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { Check, KeyRound, LogOut } from "lucide-react";
import { logOut } from "@/api/auth";
import { personName } from "@/lib/identity";
import { forgetLoaded } from "@/lib/reloading";
import { cn } from "@/lib/utils";
import { Avatar } from "./avatar";
import { usePopover } from "./popover";

// Your avatar at the far right of the bar, and its menu: who you are, your
// API keys (they reach all your workspaces), your other workspaces (when you
// have more than one), and Log out.

type Workspace = { id: string; name: string };

const item =
  "flex h-[30px] w-full items-center gap-2 rounded-control px-2 text-left whitespace-nowrap text-ink outline-none hover:bg-hover focus-visible:bg-hover disabled:cursor-not-allowed disabled:text-ink-3 [&>svg]:size-(--icon-sm) [&>svg]:shrink-0 [&>svg]:text-ink-2";

export function AccountMenu({
  user,
  workspace,
  workspaces,
}: {
  user: { id: string; email: string };
  workspace: Workspace;
  workspaces: readonly Workspace[];
}) {
  const { open, toggle, setOpen, anchor, opener } = usePopover();
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const name = personName(user.email);

  async function logOutNow() {
    setBusy(true);
    try {
      await logOut();
    } catch {
      // A session that is already over is what logging out is after.
    }
    forgetLoaded();
    await navigate({ to: "/login" });
  }

  return (
    <div ref={anchor} className="relative inline-flex">
      <button
        ref={opener}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`${name}, account`}
        onClick={toggle}
        className="grid size-8 place-items-center rounded-full transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none aria-expanded:bg-hover"
      >
        <Avatar name={name} tintKey={user.id} size="md" />
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Account"
          onClick={(event) => {
            if ((event.target as HTMLElement).closest("a")) setOpen(false);
          }}
          className="absolute top-[calc(100%+4px)] right-0 z-40 min-w-[220px] origin-top-right animate-pop-in rounded-card bg-sheet p-1 shadow-pop"
        >
          <div className="px-2 pt-1.5 pb-1 text-meta font-medium text-ink-3">
            {name} · {workspace.name}
          </div>
          <Link role="menuitem" to="/w/$workspaceId/api-keys" params={{ workspaceId: workspace.id }} className={item}>
            <KeyRound aria-hidden strokeWidth={2.25} />
            API keys
          </Link>
          {workspaces.length > 1 && (
            <>
              <div aria-hidden className="-mx-1 my-1 h-px bg-line" />
              <div className="px-2 pt-1 pb-1 text-meta font-medium text-ink-3">Workspaces</div>
              {workspaces.map((candidate) => (
                <Link
                  key={candidate.id}
                  role="menuitem"
                  to="/w/$workspaceId"
                  params={{ workspaceId: candidate.id }}
                  activeProps={{}}
                  aria-current={candidate.id === workspace.id ? "true" : undefined}
                  className={cn(item, candidate.id === workspace.id && "font-medium")}
                >
                  <span className="min-w-0 flex-1 truncate">{candidate.name}</span>
                  {candidate.id === workspace.id && <Check aria-hidden strokeWidth={2.25} />}
                </Link>
              ))}
            </>
          )}
          <div aria-hidden className="-mx-1 my-1 h-px bg-line" />
          <button type="button" role="menuitem" disabled={busy} onClick={() => void logOutNow()} className={item}>
            <LogOut aria-hidden strokeWidth={2.25} />
            Log out
          </button>
        </div>
      )}
    </div>
  );
}
