// US-039: the web app is styled with Tailwind only, through shadcn/ui. shadcn's
// configuration (web/components.json) names the one stylesheet of the app,
// which loads Tailwind. The only other stylesheet under src/ is the shared
// stylesheet of HTML docs (US-042, src/viewer/doc.css): it styles doc content
// inside the sandboxed viewer frame and the export, never the app, so the app
// reads it only as text (?raw or ?inline) and never loads it as a stylesheet.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "vitest";

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
  for (const load of loads) {
    expect(load, "doc.css is read as text, never loaded into the app").toMatch(/\?(raw|inline)$/);
  }
});
