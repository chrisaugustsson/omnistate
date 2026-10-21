import { useState, type FormEvent } from "react";
import { createFileRoute, redirect, useNavigate } from "@tanstack/react-router";
import { loadSession, logIn } from "@/api/auth";
import { ApiError } from "@/api/client";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
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
    <main className="flex min-h-screen items-center justify-center p-4">
      <Card className="w-full max-w-sm">
        <CardHeader>
          <CardTitle>OmniState</CardTitle>
          <CardDescription>Log in to review your projects.</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={submit} className="grid gap-4">
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
                value={password}
                onChange={(event) => setPassword(event.target.value)}
              />
            </div>
            <Button type="submit" disabled={busy}>
              Log in
            </Button>
          </form>
        </CardContent>
      </Card>
    </main>
  );
}
