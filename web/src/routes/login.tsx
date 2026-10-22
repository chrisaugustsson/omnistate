import { useState, type FormEvent } from "react";
import { createFileRoute, redirect, useNavigate } from "@tanstack/react-router";
import { loadSession, logIn } from "@/api/auth";
import { ApiError } from "@/api/client";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export const Route = createFileRoute("/login")({
  ssr: false,
  beforeLoad: async () => {
    const session = await loadSession();
    const [workspace] = session?.workspaces ?? [];
    if (workspace) {
      throw redirect({ to: "/w/$workspaceId", params: { workspaceId: workspace.id } });
    }
  },
  component: LoginPage,
});

function refusal(error: unknown): string {
  if (error instanceof ApiError && error.status === 401) return "The email or password is wrong.";
  return "Logging in failed. Try again in a moment.";
}

function LoginPage() {
  const navigate = useNavigate();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setProblem(null);
    try {
      await logIn({ email, password });
      const session = await loadSession();
      const [workspace] = session?.workspaces ?? [];
      if (workspace) {
        await navigate({ to: "/w/$workspaceId", params: { workspaceId: workspace.id } });
      } else {
        await navigate({ to: "/" });
      }
    } catch (error) {
      setProblem(refusal(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="grid min-h-dvh bg-background lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
      <div className="flex flex-col px-6 py-8 sm:px-12">
        <div className="flex items-center gap-2.5">
          <span
            aria-hidden
            className="flex size-7 items-center justify-center rounded-lg bg-primary font-display text-lg leading-none text-primary-foreground"
          >
            O
          </span>
          <span className="text-sm font-semibold">OmniState</span>
        </div>
        <div className="mx-auto flex w-full max-w-sm flex-1 flex-col justify-center py-12">
          <h1 className="font-display text-[2.5rem] leading-[1.05]">Welcome back</h1>
          <p className="mt-2 text-[15px] text-muted-foreground">Log in to review your projects.</p>
          <form onSubmit={submit} className="mt-8 grid gap-5">
            {problem && (
              <Alert variant="destructive">
                <AlertDescription>{problem}</AlertDescription>
              </Alert>
            )}
            <div className="grid gap-2">
              <Label htmlFor="email">Email</Label>
              <Input
                id="email"
                type="email"
                autoComplete="username"
                required
                className="h-10"
                value={email}
                onChange={(event) => setEmail(event.target.value)}
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="password">Password</Label>
              <Input
                id="password"
                type="password"
                autoComplete="current-password"
                required
                className="h-10"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
              />
            </div>
            <Button type="submit" size="lg" disabled={busy}>
              Log in
            </Button>
          </form>
        </div>
      </div>
      <Showcase />
    </main>
  );
}

/** The right half of the login page: what OmniState is, shown as a decision waiting for the reviewer. */
function Showcase() {
  return (
    <div aria-hidden className="relative hidden overflow-hidden bg-canvas lg:block">
      <div className="absolute inset-0 bg-[radial-gradient(circle,oklch(0.2_0.01_60/0.09)_1px,transparent_1px)] [background-size:20px_20px]" />
      <div className="relative flex h-full flex-col justify-center px-16">
        <p className="max-w-md font-display text-5xl leading-[1.05] text-foreground">
          Agents write the plans. <span className="text-signal italic">You decide.</span>
        </p>
        <div className="mt-12 max-w-md space-y-3">
          <div className="rotate-[-1.5deg] rounded-xl border bg-background p-4 shadow-panel">
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <span className="size-1.5 rounded-full bg-signal" />
              Decision to approve
            </div>
            <p className="mt-2 text-sm font-medium">Use a bottom tab bar on phones; counts show as badges.</p>
            <div className="mt-3 flex items-center gap-2">
              <span className="rounded-md bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground">Approve</span>
              <span className="rounded-md border px-2.5 py-1 text-xs font-medium">Reject</span>
            </div>
          </div>
          <div className="ml-10 rotate-[1deg] rounded-xl border bg-background p-4 shadow-panel">
            <div className="font-mono text-[11px] text-muted-foreground">explorer:accessibility · phase 2</div>
            <p className="mt-1.5 text-sm">Tap targets in the day board are 32 px; the criteria ask for 44.</p>
          </div>
        </div>
      </div>
    </div>
  );
}
