// @vitest-environment jsdom
// useReloading with a key: a page that shows the same thing again starts from
// what was loaded last, so moving between pages does not flash an empty page
// for a frame; the reload that follows replaces it. Without a key, or under
// another key, nothing kept is shown.
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { useReloading } from "./lib/reloading";

afterEach(cleanup);

/** A load whose answer the test gives: `answer(value)`. */
function deferred() {
  let answer: (value: string) => void = () => {};
  const load = () => new Promise<string>((done) => (answer = done));
  return { load, answer: (value: string) => answer(value) };
}

function Shows({ load, name, seen }: { load: () => Promise<string>; name?: string; seen?: string[] }) {
  const { value } = useReloading(load, name);
  seen?.push(`${name}: ${value ?? "nothing yet"}`);
  return <p>{value ?? "nothing yet"}</p>;
}

test("a second page under the same key shows the last value at once, then the new one", async () => {
  const first = deferred();
  const { unmount } = render(<Shows load={first.load} name="home:p1" />);
  first.answer("three tracks");
  await screen.findByText("three tracks");
  unmount();

  const seen: string[] = [];
  const second = deferred();
  render(<Shows load={second.load} name="home:p1" seen={seen} />);
  expect(seen[0]).toBe("home:p1: three tracks");
  second.answer("four tracks");
  await screen.findByText("four tracks");
});

test("another key, or none, starts from nothing", async () => {
  const first = deferred();
  const { unmount } = render(<Shows load={first.load} name="home:p1" />);
  first.answer("three tracks");
  await screen.findByText("three tracks");
  unmount();

  const { unmount: other } = render(<Shows load={deferred().load} name="home:p2" />);
  expect(screen.getByText("nothing yet")).toBeTruthy();
  other();
  render(<Shows load={deferred().load} />);
  expect(screen.getByText("nothing yet")).toBeTruthy();
});

test("when the key changes on a mounted page, it shows what is kept under the new key, never the old key's value", async () => {
  const one = deferred();
  const { rerender } = render(<Shows load={one.load} name="home:p1" />);
  one.answer("project one");
  await screen.findByText("project one");

  // Every render is logged: a render between the key change and the effect would be a painted frame.
  const seen: string[] = [];
  const two = deferred();
  rerender(<Shows load={two.load} name="home:p2" seen={seen} />);
  two.answer("project two");
  await waitFor(() => expect(screen.getByText("project two")).toBeTruthy());
  expect(seen.filter((line) => line.includes("project one"))).toEqual([]);
});
