// US-039: the web app is styled with Tailwind only, through shadcn/ui. shadcn's
// configuration (web/components.json) names the one stylesheet of the app,
// which loads Tailwind; there is no other stylesheet under src/.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "vitest";

const webDir = fileURLToPath(new URL("..", import.meta.url));
const srcDir = join(webDir, "src");

function readJson(path: string): Record<string, unknown> {
  expect(existsSync(path), `${relative(webDir, path)} exists`).toBe(true);
  return JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
}

function filesUnder(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory() ? filesUnder(join(dir, entry.name)) : [join(dir, entry.name)],
  );
}

test("shadcn/ui is set up for Tailwind and its stylesheet loads Tailwind", () => {
  const config = readJson(join(webDir, "components.json"));
  const tailwind = config.tailwind as { css?: string } | undefined;
  expect(typeof tailwind?.css, "components.json names the Tailwind stylesheet").toBe("string");
  const stylesheet = resolve(webDir, tailwind!.css!);
  expect(existsSync(stylesheet), `${tailwind!.css} exists`).toBe(true);
  expect(readFileSync(stylesheet, "utf8")).toMatch(/@import\s+["']tailwindcss["']/);

  const pkg = readJson(join(webDir, "package.json"));
  const deps = {
    ...(pkg.dependencies as Record<string, string> | undefined),
    ...(pkg.devDependencies as Record<string, string> | undefined),
  };
  expect(Object.keys(deps)).toContain("tailwindcss");
});

test("the Tailwind stylesheet is the only stylesheet of the app", () => {
  const config = readJson(join(webDir, "components.json"));
  const tailwind = config.tailwind as { css?: string } | undefined;
  expect(typeof tailwind?.css).toBe("string");
  const stylesheet = resolve(webDir, tailwind!.css!);
  const stylesheets = filesUnder(srcDir).filter((file) => /\.(css|scss|sass|less)$/.test(file));
  expect(stylesheets.map((file) => relative(webDir, file))).toEqual([relative(webDir, stylesheet)]);
});
