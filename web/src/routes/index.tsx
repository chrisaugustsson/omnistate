import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/")({
  component: Home,
});

function Home() {
  return (
    <main>
      <h1>OmniState</h1>
      <p>The stack is running.</p>
    </main>
  );
}
