// The images and fonts a design draws with. A design names its images as
// `attachment:<id>` (an image node's `image`, and the `image` of a fill of type
// "image"), on its pages and in the components its instances place. Its design
// system attaches font files to its typography tokens, in
// `$extensions.omnistate.fontFiles` (docs/formats/tokens.md). All of them are
// workspace attachments. The renderer takes their bytes through `addImage` and
// `addFont`, and draws a page with the ones it was given before it showed it.

import { attachmentId, loadAttachment } from "@/api/attachments";
import type { DesignRenderer } from "@/design/renderer";

type Json = Record<string, unknown>;

const isObject = (value: unknown): value is Json => typeof value === "object" && value !== null && !Array.isArray(value);

/**
 * Every `attachment:<id>` image that `page` shows: its own, and those of the
 * components it places. A component is placed by an instance's `component`, by
 * an override that swaps it or by a slot, which all name its ID; the other
 * variants of its set are placed when a property chooses them.
 */
export function imagesOf(page: unknown, components: unknown): string[] {
  const listed = (Array.isArray(components) ? components : []).filter(isObject);
  const byId = new Map(listed.map((component) => [String(component.id), component]));
  const byName = new Map<string, Json[]>();
  for (const component of listed) {
    const name = String(component.name);
    byName.set(name, [...(byName.get(name) ?? []), component]);
  }
  const found = new Set<string>();
  const entered = new Set<string>();

  const enter = (component: Json) => {
    const id = String(component.id);
    if (entered.has(id)) return;
    entered.add(id);
    visit(component);
    for (const sibling of byName.get(String(component.name)) ?? []) enter(sibling);
  };
  const visit = (value: unknown) => {
    if (typeof value === "string") {
      const component = byId.get(value);
      if (component) enter(component);
    } else if (Array.isArray(value)) {
      value.forEach(visit);
    } else if (isObject(value)) {
      for (const [key, child] of Object.entries(value)) {
        if (key === "image" && typeof child === "string" && attachmentId(child) !== null) found.add(child);
        else visit(child);
      }
    }
  };
  visit(page);
  return Array.from(found);
}

/** The font files of the typography tokens of `tokens`, each with the family of the token that lists it. */
export function fontsOf(tokens: unknown): { family: string; attachment: string }[] {
  const found = new Map<string, { family: string; attachment: string }>();
  const visit = (node: unknown) => {
    if (!isObject(node)) return;
    if ("$value" in node) {
      const family = isObject(node.$value) ? node.$value.fontFamily : undefined;
      const omnistate = isObject(node.$extensions) ? node.$extensions.omnistate : undefined;
      const files = isObject(omnistate) ? omnistate.fontFiles : undefined;
      if (typeof family === "string" && Array.isArray(files)) {
        for (const file of files) {
          if (typeof file === "string" && attachmentId(file) !== null) {
            found.set(`${family}\n${file}`, { family, attachment: file });
          }
        }
      }
      return;
    }
    for (const [name, child] of Object.entries(node)) {
      if (!name.startsWith("$")) visit(child);
    }
  };
  visit(tokens);
  return Array.from(found.values());
}

export type AssetLoader = {
  /**
   * Gives the renderer the images of `page` and the font files of `system`
   * (the design system as `show` takes it) that it does not have yet, so that
   * `show` draws with them. Resolves with a sentence for each image or font
   * that could not be loaded, and the rest is loaded all the same.
   */
  load(page: unknown, system: unknown): Promise<string[]>;
};

/** An asset loader for one renderer, which remembers what the renderer was given. */
export function createAssetLoader(renderer: DesignRenderer, workspaceId: string): AssetLoader {
  // Each asset is loaded once at a time; one that failed is tried again by the next load.
  const jobs = new Map<string, Promise<string | null>>();

  function job(key: string, attachment: string, what: string, add: (bytes: Uint8Array) => void) {
    const running = jobs.get(key);
    if (running) return running;
    const started = loadAttachment(workspaceId, attachmentId(attachment) ?? attachment).then(
      (bytes) => {
        try {
          add(bytes);
          return null;
        } catch (error) {
          return `${what} could not be read by the renderer${reason(error)}.`;
        }
      },
      (error: unknown) => `${what} could not be loaded${reason(error)}.`,
    );
    jobs.set(key, started);
    void started.then((problem) => {
      if (problem !== null && jobs.get(key) === started) jobs.delete(key);
    });
    return started;
  }

  return {
    async load(page, system) {
      const components = isObject(system) ? system.$components : undefined;
      const images = imagesOf(page, components).map((image) =>
        job(image, image, `The image ${image}`, (bytes) => renderer.addImage(image, bytes)),
      );
      const fonts = fontsOf(system).map(({ family, attachment }) =>
        job(`${family}\n${attachment}`, attachment, `The font file ${attachment} of ${family}`, (bytes) =>
          renderer.addFont(family, bytes),
        ),
      );
      const problems = await Promise.all([...images, ...fonts]);
      return problems.filter((problem): problem is string => problem !== null);
    },
  };
}

function reason(error: unknown): string {
  return error instanceof Error && error.message ? ` (${error.message})` : "";
}
