import { createDesignRenderer, type DesignRenderer } from "./renderer";

// Small pictures of nodes for the comments of a design: the node in tag form,
// drawn by the renderer, and for the activity of a design system a component,
// drawn on a board of its own. One renderer on a hidden canvas draws the nodes
// and one the components (a design system stays with a renderer), one after
// the other, and each picture is copied onto the canvas of its place, so that
// a list of pictures does not hold a WebGL context for each.

/** The size of a picture in CSS pixels. */
export const THUMBNAIL = { width: 160, height: 96 };

type Drawing = { canvas: HTMLCanvasElement; renderer: DesignRenderer };

const drawings: { nodes: Promise<Drawing> | null; components: Promise<Drawing> | null } = {
  nodes: null,
  components: null,
};
let queue: Promise<unknown> = Promise.resolve();

function sharedRenderer(which: keyof typeof drawings = "nodes"): Promise<Drawing> {
  const drawing = (drawings[which] ??= (async () => {
    const canvas = document.createElement("canvas");
    canvas.setAttribute("aria-hidden", "true");
    canvas.style.cssText = `position:fixed;left:-10000px;top:0;width:${THUMBNAIL.width}px;height:${THUMBNAIL.height}px;pointer-events:none`;
    document.body.append(canvas);
    try {
      return { canvas, renderer: await createDesignRenderer(canvas) };
    } catch (error) {
      canvas.remove();
      throw error;
    }
  })());
  drawing.catch(() => {
    drawings[which] = null;
  });
  return drawing;
}

/**
 * Draws the node in `tags` onto `target`, a canvas of `size` in CSS pixels
 * (`THUMBNAIL`'s unless given). Rejects when the renderer cannot start or the
 * tags are not a node that it can draw.
 */
export function drawSnapshot(tags: string, target: HTMLCanvasElement, size = THUMBNAIL): Promise<void> {
  const draw = async () => {
    const { canvas, renderer } = await sharedRenderer();
    canvas.style.width = `${size.width}px`;
    canvas.style.height = `${size.height}px`;
    renderer.showTags(tags);
    copy(canvas, target);
  };
  return enqueue(draw);
}

function enqueue(draw: () => Promise<void>): Promise<void> {
  const done = queue.then(draw);
  queue = done.catch(() => undefined);
  return done;
}

let shown = 0;

/**
 * Draws the component `node` (a component of a design system, as JSON) onto
 * `target`, a canvas of `size` in CSS pixels, as an instance of it on a board
 * of its own, resolved against `system` (the design system's tokens with its
 * components under `$components`, the component among them). Rejects when the
 * renderer cannot start or cannot draw it.
 */
export function drawComponent(
  node: Record<string, unknown>,
  system: Record<string, unknown>,
  target: HTMLCanvasElement,
  size = THUMBNAIL,
): Promise<void> {
  const draw = async () => {
    const { canvas, renderer } = await sharedRenderer("components");
    canvas.style.width = `${size.width}px`;
    canvas.style.height = `${size.height}px`;
    // Each picture is on a page of its own, so that its view is fitted afresh.
    shown += 1;
    const id = (prefix: string) => `${prefix}${String(shown).padStart(6, "0")}`;
    renderer.show(
      {
        id: id("cp"),
        type: "page",
        name: "Component",
        children: [
          {
            id: id("cb"),
            type: "board",
            name: "Component",
            x: 0,
            y: 0,
            width: "hug",
            height: "hug",
            layout: { mode: "flex", direction: "column" },
            children: [{ id: id("cn"), type: "instance", name: "Component", component: node.id }],
          },
        ],
      },
      system,
    );
    // Reading the boards in view draws the page's first frame.
    renderer.visibleBoards();
    copy(canvas, target);
  };
  return enqueue(draw);
}

function copy(canvas: HTMLCanvasElement, target: HTMLCanvasElement) {
  const context = target.getContext("2d");
  if (!context) throw new Error("this browser cannot draw a picture");
  target.width = canvas.width;
  target.height = canvas.height;
  context.drawImage(canvas, 0, 0);
}
