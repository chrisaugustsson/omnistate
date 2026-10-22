// The design renderer: omni-render (Skia and Taffy, built to WebAssembly by
// `make render-wasm`) drawing a design's page on a canvas through WebGL. The
// design viewer reaches the renderer only through this module.
//
// The module and its .wasm are served from /render/, where `make render-wasm`
// copies them (web/public/render/, not committed). The WebAssembly module
// takes text as bytes in its own memory: the page copies it in with
// `_omni_render_alloc` and gives the room back with `_omni_render_free`.

export type Layer = { address: string; name: string; type: string; children: Layer[] };

export type TokenUse = { token: string; value: unknown };
export type Property = { name: string; value: unknown; tokens: TokenUse[] };

/** A node as the inspect panel shows it (omni-render's `_omni_render_inspect`). */
export type Inspection = {
  address: string;
  name: string;
  type: string;
  /** The mode the tokens are resolved in; `null` for a design without a design system. */
  mode: string | null;
  /** The node's laid-out box in page coordinates. */
  box: { x: number; y: number; width: number; height: number };
  layout: Property[];
  size: Property[];
  style: Property[];
  text: Property[];
  /** For an instance: its component, property values and overrides. */
  instance: null | {
    component: string;
    componentName: string;
    props: { name: string; kind: string; value: unknown; set: boolean }[];
    overrides: { node: string; name: string; fields: Record<string, unknown> }[];
  };
};

export type DesignRenderer = {
  /**
   * Shows `page` (one of a version's `pages`, as the API gives it), resolving
   * its instances and token references against `system`: the version's token
   * tree with its components under `$components`.
   */
  show(page: unknown, system: unknown): void;
  /** The shown page's boards with every node below them. */
  layers(): Layer[];
  /** Selects the node at `address` on the canvas; `null` clears the selection. */
  select(address: string | null): void;
  /**
   * Resolves the shown page's tokens in `mode` from now on and draws it again.
   * `show` resolves a page in the first mode of its design system, so call
   * this again after showing another page in another mode.
   */
  setMode(mode: string): void;
  /** The node at `address` in the current mode, or `null` when no node has that address. */
  inspect(address: string): Inspection | null;
  /** Calls `listener` when the reviewer selects a node on the canvas; returns the function that stops it. */
  onSelect(listener: (address: string | null) => void): () => void;
  destroy(): void;
};

const MODULE_URL = "/render/omni_render.js";

type Outline = { kind: "hover" | "selection"; node: string; label: string; x: number; y: number; width: number; height: number };
type RenderEvent = { type: "hover" | "select"; node: string | null };

/** The functions of the WebAssembly module that the viewer calls. */
type OmniRender = {
  HEAPU8: Uint8Array;
  UTF8ToString(pointer: number): string;
  GL: {
    registerContext(gl: WebGL2RenderingContext, attributes: { majorVersion: number; minorVersion: number }): number;
    makeContextCurrent(context: number): boolean;
  };
  _omni_render_alloc(length: number): number;
  _omni_render_free(pointer: number, length: number): void;
  _omni_render_error(): number;
  _omni_render_scene_load(pointer: number, length: number): boolean;
  _omni_render_scene_set_system(pointer: number, length: number): boolean;
  _omni_render_scene_set_mode(pointer: number, length: number): boolean;
  _omni_render_inspect(pointer: number, length: number): number;
  _omni_render_scene_draw(width: number, height: number, pixelRatio: number): boolean;
  _omni_render_scene_layers(): number;
  _omni_render_select_node(pointer: number, length: number): boolean;
  _omni_render_select_pointer_move(x: number, y: number, ctrl: boolean, meta: boolean): void;
  _omni_render_select_click(x: number, y: number, ctrl: boolean, meta: boolean): void;
  _omni_render_select_double_click(x: number, y: number): void;
  _omni_render_select_escape(): void;
  _omni_render_take_events(): number;
  _omni_render_overlay(): number;
  _omni_render_view_wheel(dx: number, dy: number, x: number, y: number, ctrl: boolean, meta: boolean): boolean;
  _omni_render_view_pointer_down(x: number, y: number): boolean;
  _omni_render_view_pointer_move(x: number, y: number): boolean;
  _omni_render_view_pointer_up(): void;
  _omni_render_view_space(held: boolean): void;
};

let factory: Promise<() => Promise<OmniRender>> | null = null;

function moduleFactory() {
  // The URL is a variable so that the bundler leaves the import to the browser.
  factory ??= import(/* @vite-ignore */ MODULE_URL).then(
    (loaded: { default: () => Promise<OmniRender> }) => loaded.default,
    (error) => {
      factory = null;
      throw error;
    },
  );
  return factory;
}

export async function createDesignRenderer(canvas: HTMLCanvasElement): Promise<DesignRenderer> {
  // Skia keeps its GPU context for the page's WebGL context, so every
  // renderer has a module instance of its own.
  const module = await (await moduleFactory())();
  const encoder = new TextEncoder();
  const failure = () => new Error(module.UTF8ToString(module._omni_render_error()));
  const withText = <T>(text: string, use: (pointer: number, length: number) => T): T => {
    const bytes = encoder.encode(text);
    const pointer = module._omni_render_alloc(bytes.length);
    // HEAPU8 is looked up after the allocation, which may grow the memory.
    module.HEAPU8.set(bytes, pointer);
    try {
      return use(pointer, bytes.length);
    } finally {
      module._omni_render_free(pointer, bytes.length);
    }
  };

  // omni-render draws on the default framebuffer of the current WebGL
  // context: WebGL 2 with an alpha channel, an 8-bit stencil buffer and no
  // multisampling (Skia anti-aliases by itself).
  const gl = canvas.getContext("webgl2", { alpha: true, depth: false, stencil: true, antialias: false });
  if (!gl) throw new Error("this browser has no WebGL 2");
  module.GL.makeContextCurrent(module.GL.registerContext(gl, { majorVersion: 2, minorVersion: 0 }));

  // Hover and selection outlines are elements over the canvas.
  const overlay = document.createElement("div");
  overlay.setAttribute("aria-hidden", "true");
  overlay.style.cssText = "position:absolute;inset:0;overflow:hidden;pointer-events:none";
  canvas.insertAdjacentElement("afterend", overlay);

  const listeners = new Set<(address: string | null) => void>();
  let loaded = false;
  let dirty = true;
  let frame = 0;
  let destroyed = false;
  let pointer: [number, number] | null = null;
  let keys = { ctrl: false, meta: false };

  const readJson = <T>(pointerToText: number): T => JSON.parse(module.UTF8ToString(pointerToText)) as T;

  function drawOverlay() {
    const outlines = readJson<Outline[]>(module._omni_render_overlay());
    overlay.replaceChildren(
      ...outlines.map((outline) => {
        const box = document.createElement("div");
        const color = outline.kind === "selection" ? "#db2777" : "#3b82f6";
        box.style.cssText =
          `position:absolute;box-sizing:border-box;border:${outline.kind === "selection" ? 2 : 1.5}px solid ${color};` +
          `left:${outline.x}px;top:${outline.y}px;width:${outline.width}px;height:${outline.height}px`;
        const label = document.createElement("span");
        label.textContent = outline.label;
        label.style.cssText =
          `position:absolute;left:-2px;${outline.y < 18 ? "top:0" : "bottom:100%"};padding:1px 6px;background:${color};` +
          "color:#fff;font:11px system-ui,sans-serif;white-space:nowrap";
        box.append(label);
        return box;
      }),
    );
  }

  /** Tells the listeners which node the reviewer selected on the canvas. */
  function takeEvents() {
    const events = readJson<RenderEvent[]>(module._omni_render_take_events());
    for (const event of events) {
      if (event.type !== "select") continue;
      for (const listener of Array.from(listeners)) listener(event.node);
    }
    drawOverlay();
  }

  function drawFrame() {
    const ratio = window.devicePixelRatio || 1;
    const width = Math.max(1, Math.round(canvas.clientWidth * ratio));
    const height = Math.max(1, Math.round(canvas.clientHeight * ratio));
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
    if (!module._omni_render_scene_draw(canvas.width, canvas.height, ratio)) throw failure();
    drawOverlay();
  }

  function requestFrame() {
    dirty = true;
  }

  function loop() {
    if (destroyed) return;
    if (dirty && loaded) {
      dirty = false;
      drawFrame();
    }
    frame = requestAnimationFrame(loop);
  }

  const position = (event: { clientX: number; clientY: number }): [number, number] => {
    const rect = canvas.getBoundingClientRect();
    return [event.clientX - rect.left, event.clientY - rect.top];
  };

  function hover() {
    if (pointer) module._omni_render_select_pointer_move(pointer[0], pointer[1], keys.ctrl, keys.meta);
  }

  // Whether the click that follows the pointer going up ended a pan.
  let ignoreClick = false;

  const onWheel = (event: WheelEvent) => {
    if (!loaded) return;
    const [x, y] = position(event);
    const unit = [1, 16, canvas.clientHeight][event.deltaMode] ?? 1;
    // Ctrl + wheel is also how browsers report a trackpad pinch.
    if (module._omni_render_view_wheel(event.deltaX * unit, event.deltaY * unit, x, y, event.ctrlKey, event.metaKey)) {
      event.preventDefault();
      requestFrame();
    }
  };
  const onPointerDown = (event: PointerEvent) => {
    if (!loaded) return;
    const [x, y] = position(event);
    canvas.setPointerCapture?.(event.pointerId);
    ignoreClick = false;
    if (module._omni_render_view_pointer_down(x, y)) ignoreClick = true;
  };
  const onPointerMove = (event: PointerEvent) => {
    if (!loaded) return;
    const [x, y] = position(event);
    if (module._omni_render_view_pointer_move(x, y)) {
      requestFrame();
      return;
    }
    pointer = [x, y];
    keys = { ctrl: event.ctrlKey, meta: event.metaKey };
    hover();
    takeEvents();
  };
  const onPointerUp = () => {
    if (loaded) module._omni_render_view_pointer_up();
  };
  const onPointerLeave = () => {
    if (!loaded) return;
    pointer = null;
    // A point that is nowhere, so that hover clears.
    module._omni_render_select_pointer_move(NaN, NaN, false, false);
    takeEvents();
  };
  const onClick = (event: MouseEvent) => {
    if (!loaded) return;
    if (ignoreClick) {
      ignoreClick = false;
      return;
    }
    const [x, y] = position(event);
    module._omni_render_select_click(x, y, event.ctrlKey, event.metaKey);
    // The context changed, so hover is what a click would select now.
    pointer = [x, y];
    hover();
    takeEvents();
  };
  const onDoubleClick = (event: MouseEvent) => {
    if (!loaded) return;
    const [x, y] = position(event);
    module._omni_render_select_double_click(x, y);
    pointer = [x, y];
    hover();
    takeEvents();
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (!loaded) return;
    keys = { ctrl: event.ctrlKey, meta: event.metaKey };
    const typing = event.target instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT)$/.test(event.target.tagName);
    if (event.key === "Escape" && !typing) {
      module._omni_render_select_escape();
      hover();
      takeEvents();
    }
    // Space held lets the pointer pan, and would scroll the page.
    if (event.code === "Space" && !typing && pointer) {
      event.preventDefault();
      module._omni_render_view_space(true);
    }
  };
  const onKeyUp = (event: KeyboardEvent) => {
    if (!loaded) return;
    keys = { ctrl: event.ctrlKey, meta: event.metaKey };
    if (event.code === "Space") module._omni_render_view_space(false);
  };
  const onBlur = () => {
    if (loaded) module._omni_render_view_space(false);
  };

  // The wheel listener is not passive, so that preventDefault can keep the
  // browser from zooming or scrolling the page.
  canvas.addEventListener("wheel", onWheel, { passive: false });
  canvas.addEventListener("pointerdown", onPointerDown);
  canvas.addEventListener("pointermove", onPointerMove);
  canvas.addEventListener("pointerup", onPointerUp);
  canvas.addEventListener("pointercancel", onPointerUp);
  canvas.addEventListener("pointerleave", onPointerLeave);
  canvas.addEventListener("click", onClick);
  canvas.addEventListener("dblclick", onDoubleClick);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("keyup", onKeyUp);
  window.addEventListener("blur", onBlur);
  const resizing = new ResizeObserver(requestFrame);
  resizing.observe(canvas);
  frame = requestAnimationFrame(loop);

  return {
    show(page, system) {
      const ok = withText(JSON.stringify(system), (pointer, length) => module._omni_render_scene_set_system(pointer, length));
      if (!ok) throw failure();
      if (!withText(JSON.stringify(page), (pointer, length) => module._omni_render_scene_load(pointer, length))) {
        throw failure();
      }
      loaded = true;
      requestFrame();
    },
    layers() {
      return loaded ? readJson<Layer[]>(module._omni_render_scene_layers()) : [];
    },
    select(address) {
      if (!loaded) return;
      withText(address ?? "", (pointer, length) => module._omni_render_select_node(pointer, length));
      // The caller knows about this selection, so it is not announced.
      module._omni_render_take_events();
      drawOverlay();
    },
    setMode(mode) {
      if (!loaded) return;
      if (!withText(mode, (pointer, length) => module._omni_render_scene_set_mode(pointer, length))) throw failure();
      requestFrame();
    },
    inspect(address) {
      if (!loaded) return null;
      const text = withText(address, (pointer, length) => module._omni_render_inspect(pointer, length));
      return text ? readJson<Inspection>(text) : null;
    },
    onSelect(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    destroy() {
      destroyed = true;
      cancelAnimationFrame(frame);
      resizing.disconnect();
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("pointerdown", onPointerDown);
      canvas.removeEventListener("pointermove", onPointerMove);
      canvas.removeEventListener("pointerup", onPointerUp);
      canvas.removeEventListener("pointercancel", onPointerUp);
      canvas.removeEventListener("pointerleave", onPointerLeave);
      canvas.removeEventListener("click", onClick);
      canvas.removeEventListener("dblclick", onDoubleClick);
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onBlur);
      overlay.remove();
      listeners.clear();
    },
  };
}
