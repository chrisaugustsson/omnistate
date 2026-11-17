// US-039: the web app is styled with Tailwind only, through shadcn/ui. shadcn's
// configuration (web/components.json) names the one stylesheet of the app,
// which loads Tailwind. The only other stylesheet under src/ is the shared
// stylesheet of HTML docs (US-042, src/viewer/doc.css): it styles doc content
// inside the sandboxed viewer frame and the export, never the app, so the app
// reads it only as text (?raw or ?inline) and never loads it as a stylesheet.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";

const webDir = fileURLToPath(new URL("..", import.meta.url));
const srcDir = join(webDir, "src");
const docStylesheet = join(srcDir, "viewer", "doc.css");

function readJson(path: string): Record<string, unknown> {
  expect(existsSync(path), `${relative(webDir, path)} exists`).toBe(true);
  return JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
}

function filesUnder(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory() ? filesUnder(join(dir, entry.name)) : [join(dir, entry.name)],
  );
}

test("the Tailwind stylesheet is the only stylesheet of the app", () => {
  const config = readJson(join(webDir, "components.json"));
  const tailwind = config.tailwind as { css?: string } | undefined;
  expect(typeof tailwind?.css).toBe("string");
  const stylesheet = resolve(webDir, tailwind!.css!);
  const stylesheets = filesUnder(srcDir).filter((file) => /\.(css|scss|sass|less)$/.test(file));
  expect(stylesheets.map((file) => relative(webDir, file)).sort()).toEqual(
    [relative(webDir, stylesheet), relative(webDir, docStylesheet)].sort(),
  );
});

test("the HTML doc stylesheet is doc content styling, not app styling", () => {
  expect(existsSync(docStylesheet), "src/viewer/doc.css exists").toBe(true);
  expect(readFileSync(docStylesheet, "utf8")).not.toMatch(/@import|tailwind/i);

  const sources = filesUnder(srcDir).filter((file) => /\.(ts|tsx)$/.test(file) && !/\.test\.tsx?$/.test(file));
  const loads = sources.flatMap((file) =>
    [...readFileSync(file, "utf8").matchAll(/(?:import|from)\s*\(?\s*["']([^"']*doc\.css[^"']*)["']/g)].map(
      (match) => `${relative(webDir, file)}: ${match[1]}`,
    ),
  );
  expect(loads.length, "some source reads src/viewer/doc.css").toBeGreaterThan(0);
  expect(
    loads.filter((load) => !/\?(raw|inline)$/.test(load)),
    "doc.css is read as text, never loaded into the app",
  ).toEqual([]);
});

// The UI redesign (tasks/ui-redesign/v2/direction.md, the "Tokens" table and the
// part and state colours) sets the look in tokens on :root; shadcn's variables
// point at them, so components/ui and the new shell draw in the same colours.
describe("the v2 tokens", () => {
  const css = readFileSync(join(srcDir, "styles.css"), "utf8");
  const root = css.slice(css.indexOf(":root {"));
  /** The value `name` has on :root, as written. */
  const token = (name: string) => root.match(new RegExp(`\\n\\s*${name}:\\s*([^;]+);`))?.[1].trim();

  test.each([
    ["--sheet", "#ffffff"],
    ["--hover", "#f0f0f2"],
    ["--selected", "#e9e9ec"],
    ["--line", "#ececef"],
    ["--line-strong", "#dcdce0"],
    ["--ink", "#18181b"],
    ["--ink-2", "#52525b"],
    ["--brand", "#2f5bea"],
    ["--brand-hover", "#2549c7"],
    ["--brand-tint", "#e8eefe"],
    ["--danger", "#dc2626"],
    ["--state-planning", "#64748b"],
    ["--state-building", "#d97706"],
    ["--state-done", "#16a34a"],
    ["--state-dropped", "#a1a1aa"],
    ["--part-1", "#2f5bea"],
    ["--part-2", "#0e9f6e"],
    ["--part-3", "#e8590c"],
    ["--bar-h", "48px"],
    ["--pill-h", "28px"],
    ["--fs-body", "13px"],
  ])("%s is %s", (name, value) => {
    expect(token(name)).toBe(value);
  });

  test("the app is set in Inter, and the root route loads it", () => {
    expect(token("--font")).toMatch(/^"Inter"/);
    const rootRoute = readFileSync(join(srcDir, "routes", "__root.tsx"), "utf8");
    expect(rootRoute).toContain("fonts.googleapis.com/css2?family=Inter");
  });

  test.each([
    ["--primary", "var(--brand)"],
    ["--background", "var(--sheet)"],
    ["--foreground", "var(--ink)"],
    ["--border", "var(--line)"],
    ["--ring", "var(--brand)"],
  ])("shadcn's %s is %s", (name, value) => {
    expect(token(name)).toBe(value);
  });
});
