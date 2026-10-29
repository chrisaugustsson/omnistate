import { createDesignRenderer, type DesignRenderer } from "./renderer";

// Small pictures of nodes for the comments of a design: the node in tag form,
// drawn by the renderer. One renderer on a hidden canvas draws them all, one
// after the other, and each picture is copied onto the canvas of its place, so
// that a list of snapshots does not hold a WebGL context for each.

/** The size of a picture in CSS pixels. */
export const THUMBNAIL = { width: 160, height: 96 };

type Drawing = { canvas: HTMLCanvasElement; renderer: DesignRenderer };

let drawing: Promise<Drawing> | null = null;
let queue: Promise<unknown> = Promise.resolve();

function sharedRenderer(): Promise<Drawing> {
  drawing ??= (async () => {
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
  })();
  drawing.catch(() => {
    drawing = null;
  });
  return drawing;
}

/**
 * Draws the node in `tags` onto `target`, a canvas of `THUMBNAIL`'s size in CSS
 * pixels. Rejects when the renderer cannot start or the tags are not a node
 * that it can draw.
 */
export function drawSnapshot(tags: string, target: HTMLCanvasElement): Promise<void> {
  const draw = async () => {
    const { canvas, renderer } = await sharedRenderer();
    renderer.showTags(tags);
    const context = target.getContext("2d");
    if (!context) throw new Error("this browser cannot draw a picture");
    target.width = canvas.width;
    target.height = canvas.height;
    context.drawImage(canvas, 0, 0);
  };
  const done = queue.then(draw);
  queue = done.catch(() => undefined);
  return done;
}
