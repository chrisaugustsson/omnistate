// The design renderer: omni-render (Skia and Taffy, built to WebAssembly by
// `make render-wasm`) drawing a design's page on a canvas through WebGL. The
// design viewer reaches the renderer only through this module.
//
// The module and its .wasm are served from /render/, where `make render-wasm`
// copies them (web/public/render/, not committed). The WebAssembly module
// takes text as bytes in its own memory: the page copies it in with
// `_omni_render_alloc` and gives the room back with `_omni_render_free`.

/** Texts drawn in `drawnAs` for want of a font file for the face they ask for. */
export type FontStandIn = {
  family: string;
  weight: number;
  style: string;
  drawnAs: { family: string; weight: number; style: string };
  texts: number;
};

export type Layer = { address: string; name: string; type: string; children: Layer[] };

export type TokenUse = { token: string; value: unknown };
export type Property = { name: string; value: unknown; tokens: TokenUse[] };

/** An action of an interaction; `destName` and `destKind` are left out when the target is not in the doc. */
export type InteractionAction = {
  type: string;
  dest?: string;
  destName?: string;
  destKind?: string;
  url?: string;
  newTab?: boolean;
  preserveScroll?: boolean;
  position?: string;
  offset?: [number, number];
  variable?: string;
  value?: unknown;
  expr?: unknown;
  collection?: string;
  mode?: string;
  blocks?: { condition?: unknown; actions: InteractionAction[] }[];
};

/** An interaction of a layer, with its defaults filled in (omni-render's `inspect.rs`). */
export type Interaction = {
  trigger: { type: string; ms?: number; key?: string };
  actions: InteractionAction[];
  transition: { type: string; direction?: string };
  durationMs: number;
  easing: string | { type: string; [key: string]: unknown };
  delayMs: number;
  /** Whether an action points at a node that is not in the doc. */
  dangling: boolean;
};

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
  /** The layer's own interactions. */
  interactions?: Interaction[];
  /** For a board or frame: how it scrolls its content. */
  overflow?: string | null;
  /** For a frame: `scrolls`, `fixed` or `sticky`. */
  scrollPosition?: string | null;
  /** For an instance: its component, property values and overrides. */
  instance: null | {
    component: string;
    componentName: string;
    props: { name: string; kind: string; value: unknown; set: boolean }[];
    overrides: { node: string; name: string; fields: Record<string, unknown> }[];
  };
};

/** Where a prototype that is playing is (omni-render's `play_state`). */
export type PlayState = {
  board: string;
  /** The boards left by navigates, oldest first. */
  history: string[];
  /** The boards of the open overlays, the top one last. */
  overlays: string[];
  /** Whether a click landed on nothing that reacts, given once. */
  missed: boolean;
  /** A link the prototype asked to open, given once. */
  url: { url: string; newTab: boolean } | null;
};

/** The scales the screen can be shown at on the stage: down to fit it, or at 100% or 50%. */
export type PlayScale = "fit" | "100" | "50";

/** The screen the prototype is shown on (omni-render's `play_screen`). */
export type PlayScreen = {
  /** The device shown, by its name in the device table. */
  device: string;
  scale: PlayScale;
  /** The devices to pick from, each with its screen as it would be shown; no size for `None`, which shows a board at its own size. */
  devices: { name: string; width: number | null; height: number | null }[];
};

/** The kind of a pointer event on the stage: the names, or the numbers `play_pointer` takes. */
export type PlayPointerKind = "down" | "move" | "up" | "leave" | 0 | 1 | 2 | 3;

/** A key going down or up; `key` is the name a key trigger writes: `K`, `Enter`, `ArrowLeft`, `Space`. */
export type PlayKey = { key: string; ctrl: boolean; alt: boolean; shift: boolean; cmd: boolean; down: boolean };

/** Plays the shown page's prototype on the canvas. */
export type DesignPlayer = {
  /**
   * Starts playing on `board`, a board of the page shown, in place of any
   * session: the canvas shows the board at its own size in the middle of the
   * stage, on the page's prototype background, scaled down to fit. The
   * canvas moves into `stage` for as long as the session lasts. Throws the
   * renderer's reason when the page has no such board.
   */
  start(board: string, stage?: HTMLElement): void;
  /** Ends the session and gives the canvas back to the design view. */
  stop(): void;
  /** A pointer event at the stage point `x`, `y` in CSS pixels; returns whether what is shown changed. */
  pointer(kind: PlayPointerKind, x: number, y: number): boolean;
  /** A key event; returns whether a key trigger fired, which leaves the key to the prototype. */
  key(event: PlayKey): boolean;
  /** A controller button, by its index in the standard layout, going down or up; returns whether a key trigger fired. */
  gamepad(button: number, down: boolean): boolean;
  /** Goes back to the board the last navigate left; returns whether there was one. */
  back(): boolean;
  /** A wheel turn of `dx`, `dy` CSS pixels at the stage point `x`, `y`; returns whether content scrolled. */
  wheel(x: number, y: number, dx: number, dy: number): boolean;
  /** Shows the prototype on the device `name` of `screen().devices`. Throws the renderer's reason for another name. */
  setDevice(name: string): void;
  /** Sets the scale of the screen on the stage. */
  setScale(scale: PlayScale): void;
  /** The device and the scale shown, and the devices to pick from. */
  screen(): PlayScreen;
  /** Goes to `board` as a navigate does, so that `back` returns to the board it leaves; returns whether it moved. */
  go(board: string): boolean;
  /** Starts over on `board`, with no history. */
  restart(board: string): void;
  state(): PlayState;
  /** Calls `listener` whenever what is shown changes; returns the function that stops it. */
  onChange(listener: () => void): () => void;
};

export type DesignRenderer = {
  /**
   * Supplies the image that `attachment` (`attachment:<id>`) names, from its
   * PNG or JPEG bytes. It stays with the renderer until it is destroyed.
   * Throws when the renderer cannot read the bytes.
   */
  addImage(attachment: string, bytes: Uint8Array): void;
  /**
   * Supplies the preview of `board`, a board's ID, from its PNG bytes: a small
   * picture of the board, which `usePreviews` draws it from. A renderer without
   * it draws a board that is not read as its placeholder. Throws when the
   * renderer cannot read the bytes.
   */
  addPreview?(board: string, bytes: Uint8Array): void;
  /**
   * Draws `boards` (IDs of boards of the page shown that have not been read)
   * from their previews until `updateBoards` puts them in place. A board with
   * no preview stays as it was shown.
   */
  usePreviews?(boards: string[]): void;
  /**
   * Supplies a font file (TrueType or OpenType) for text in `family`; it
   * stays until the renderer is destroyed. Throws when the renderer cannot
   * read it.
   */
  addFont(family: string, bytes: Uint8Array): void;
  /**
   * Shows `page` (one of a version's `pages`, as the API gives it), resolving
   * its instances and token references against `system`: the version's token
   * tree with its components under `$components`. The page is laid out and
   * drawn with the images and fonts supplied before this call. Showing the
   * page that is shown again (with more of its boards loaded) keeps the zoom,
   * the pan and the selection. A page opened with `board`, the ID of a board a
   * link named, opens with that board in view.
   */
  show(page: unknown, system: unknown, board?: string | null): void;
  /**
   * Queues `boards` (boards of the shown page, as the API gives them) to take
   * the place of the boards of the page that have their IDs: the way to show a
   * board that was read after the page was shown. Nothing changes until
   * `stepBoards` has done its stages. Returns `false`, with nothing queued,
   * when no page is shown or it lacks one of the boards, and the caller then
   * shows the page again. A renderer without it is shown the page again
   * instead.
   */
  updateBoards?(boards: unknown[]): boolean;
  /**
   * Does the next stage of the queued boards (resolving a board, then laying
   * it out and putting it in place: a few milliseconds each even for a big
   * board, so that a frame of the view can do one between draws) and returns
   * whether there is more to do. The zoom, the pan, the other boards' pictures
   * and the selection stay, and the next frame draws what changed. Throws when
   * the page then nests too deeply (the boards are in place in that case).
   */
  stepBoards?(): boolean;
  /**
   * Shows the node in `tags` (one tag and its subtree, the form the API gives
   * the snapshot of a comment) alone, fitted to the canvas, and draws it at
   * once. A node that is not a board sits on a board of its own size, and the
   * node's instances and token references resolve against no design system.
   * Throws when the tags are not a node.
   */
  showTags(tags: string): void;
  /**
   * The IDs of the shown page's boards that the current view shows, in page
   * order: the boards a viewer reads when they come into view. A page that was
   * just shown is drawn first, so that the answer is for its first view.
   */
  visibleBoards(): string[];
  /** The shown page's boards, each without its children. */
  layers(): Layer[];
  /** The texts of the shown page that are drawn in another font than their own, once per face asked for. */
  fontsStoodIn(): FontStandIn[];
  /** The node at `address` with every node below it; a board that is not read has no children. `null` when the page has no such node. */
  layersOf(address: string): Layer | null;
  /** The ID of the board that holds the node at `address` (a board's own ID for the board); `null` when the page has no such node. */
  boardOf(address: string): string | null;
  /**
   * Selects the node at `address` on the canvas; `null` clears the selection.
   * With `reveal`, the view moves to the node's board if the node is out of
   * view, as for a link to it.
   */
  select(address: string | null, reveal?: boolean): void;
  /**
   * Resolves the shown page's tokens in a mode from now on and draws it again:
   * a mode name, which sets the collections that inherit the root list, or a
   * mode per collection, such as `{color: "dark", density: "compact"}`, which
   * leaves the collections it does not name as they are. `show` resolves a
   * page in the first mode of each collection, so call this again after
   * showing another page in other modes.
   */
  setMode(choice: Record<string, string> | string): void;
  /** The node at `address` in the current mode, or `null` when no node has that address. */
  inspect(address: string): Inspection | null;
  /**
   * The box of the node at `address` on the canvas, in CSS pixels from the
   * canvas's top-left corner, as the current zoom and pan show it; `null`
   * when the page shown has no node at that address. `name` is the node's layer name.
   */
  boxOf(address: string): { x: number; y: number; width: number; height: number; name?: string } | null;
  /** Calls `listener` when the reviewer selects a node on the canvas; returns the function that stops it. */
  onSelect(listener: (address: string | null) => void): () => void;
  /**
   * Calls `listener` whenever the view changes: a zoom, a pan, a resize or
   * another page shown. Returns the function that stops it.
   */
  onView(listener: () => void): () => void;
  play: DesignPlayer;
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
  _omni_render_add_image(attachment: number, attachmentLength: number, data: number, dataLength: number): boolean;
  _omni_render_add_preview(board: number, boardLength: number, data: number, dataLength: number): boolean;
  _omni_render_add_font(family: number, familyLength: number, data: number, dataLength: number): boolean;
  _omni_render_scene_load(pointer: number, length: number): boolean;
  _omni_render_scene_load_tags(pointer: number, length: number): boolean;
  _omni_render_scene_set_system(pointer: number, length: number): boolean;
  _omni_render_scene_use_previews(pointer: number, length: number): boolean;
  _omni_render_scene_update_boards(pointer: number, length: number): number;
  _omni_render_scene_update_step(): number;
  _omni_render_scene_set_mode(pointer: number, length: number): boolean;
  _omni_render_inspect(pointer: number, length: number): number;
  _omni_render_node_box(pointer: number, length: number): number;
  _omni_render_scene_draw(width: number, height: number, pixelRatio: number): number;
  _omni_render_scene_layers(): number;
  _omni_render_scene_fonts(): number;
  _omni_render_scene_layers_of(pointer: number, length: number): number;
  _omni_render_scene_board_of(pointer: number, length: number): number;
  _omni_render_visible_boards(): number;
  _omni_render_select_node(pointer: number, length: number): boolean;
  _omni_render_select_pointer_move(x: number, y: number, ctrl: boolean, meta: boolean): void;
  _omni_render_select_click(x: number, y: number, ctrl: boolean, meta: boolean): void;
  _omni_render_select_double_click(x: number, y: number): void;
  _omni_render_select_escape(): void;
  _omni_render_take_events(): number;
  _omni_render_overlay(): number;
  _omni_render_view_wheel(dx: number, dy: number, x: number, y: number, ctrl: boolean, meta: boolean): boolean;
  _omni_render_view_pointer_down(x: number, y: number, middle: boolean): boolean;
  _omni_render_view_pointer_move(x: number, y: number): boolean;
  _omni_render_view_pointer_up(): void;
  _omni_render_view_space(held: boolean): void;
  _omni_render_view_open_readable(): void;
  _omni_render_view_open_on(pointer: number, length: number): boolean;
  _omni_render_view_reveal_selected(): void;
  _omni_render_play_start(board: number, boardLength: number): boolean;
  _omni_render_play_stop(): void;
  _omni_render_play_pointer(kind: number, x: number, y: number): boolean;
  _omni_render_play_key(json: number, jsonLength: number): boolean;
  _omni_render_play_gamepad(button: number, down: boolean): boolean;
  _omni_render_play_tick(now: number): number;
  _omni_render_play_back(): boolean;
  _omni_render_play_wheel(x: number, y: number, dx: number, dy: number): boolean;
  _omni_render_play_device(name: number, nameLength: number): boolean;
  _omni_render_play_scale(name: number, nameLength: number): boolean;
  _omni_render_play_screen(): number;
  _omni_render_play_go(board: number, boardLength: number): boolean;
  _omni_render_play_restart(board: number, boardLength: number): boolean;
  _omni_render_play_state(): number;
  _omni_render_play_draw(width: number, height: number, pixelRatio: number): boolean;
};

/** Every function of the module that the viewer calls, so that a module without one is caught when it loads. */
const FUNCTIONS: Record<Extract<keyof OmniRender, `_omni_render_${string}`>, true> = {
  _omni_render_alloc: true,
  _omni_render_free: true,
  _omni_render_error: true,
  _omni_render_add_image: true,
  _omni_render_add_preview: true,
  _omni_render_add_font: true,
  _omni_render_scene_load: true,
  _omni_render_scene_load_tags: true,
  _omni_render_scene_set_system: true,
  _omni_render_scene_use_previews: true,
  _omni_render_scene_update_boards: true,
  _omni_render_scene_update_step: true,
  _omni_render_scene_set_mode: true,
  _omni_render_inspect: true,
  _omni_render_node_box: true,
  _omni_render_scene_draw: true,
  _omni_render_scene_layers: true,
  _omni_render_scene_fonts: true,
  _omni_render_scene_layers_of: true,
  _omni_render_scene_board_of: true,
  _omni_render_visible_boards: true,
  _omni_render_select_node: true,
  _omni_render_select_pointer_move: true,
  _omni_render_select_click: true,
  _omni_render_select_double_click: true,
  _omni_render_select_escape: true,
  _omni_render_take_events: true,
  _omni_render_overlay: true,
  _omni_render_view_wheel: true,
  _omni_render_view_pointer_down: true,
  _omni_render_view_pointer_move: true,
  _omni_render_view_pointer_up: true,
  _omni_render_view_space: true,
  _omni_render_view_open_readable: true,
  _omni_render_view_open_on: true,
  _omni_render_view_reveal_selected: true,
  _omni_render_play_start: true,
  _omni_render_play_stop: true,
  _omni_render_play_pointer: true,
  _omni_render_play_key: true,
  _omni_render_play_gamepad: true,
  _omni_render_play_tick: true,
  _omni_render_play_back: true,
  _omni_render_play_wheel: true,
  _omni_render_play_device: true,
  _omni_render_play_scale: true,
  _omni_render_play_screen: true,
  _omni_render_play_go: true,
  _omni_render_play_restart: true,
  _omni_render_play_state: true,
  _omni_render_play_draw: true,
};

/**
 * The functions that the viewer calls and `module` lacks: a renderer built
 * before them, such as an older build that the browser kept in its cache.
 */
export function missingFunctions(module: object): string[] {
  const found = module as Record<string, unknown>;
  return Object.keys(FUNCTIONS).filter((name) => typeof found[name] !== "function");
}

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
  const missing = missingFunctions(module);
  if (missing.length > 0) {
    throw new Error(
      `The design renderer at /render/ is out of date: it has no ${missing.join(", ")}. ` +
        "Reload the page without the browser's cache, or rebuild the renderer with make render-wasm.",
    );
  }
  const encoder = new TextEncoder();
  const failure = () => new Error(module.UTF8ToString(module._omni_render_error()));
  const withBytes = <T>(bytes: Uint8Array, use: (pointer: number, length: number) => T): T => {
    const pointer = module._omni_render_alloc(bytes.length);
    // HEAPU8 is looked up after the allocation, which may grow the memory.
    module.HEAPU8.set(bytes, pointer);
    try {
      return use(pointer, bytes.length);
    } finally {
      module._omni_render_free(pointer, bytes.length);
    }
  };
  const withText = <T>(text: string, use: (pointer: number, length: number) => T): T =>
    withBytes(encoder.encode(text), use);

  // omni-render draws on the default framebuffer of the current WebGL
  // context: WebGL 2 with an alpha channel, an 8-bit stencil buffer and no
  // multisampling (Skia anti-aliases by itself).
  const gl = canvas.getContext("webgl2", { alpha: true, depth: false, stencil: true, antialias: false });
  if (!gl) throw new Error("this browser has no WebGL 2");
  module.GL.makeContextCurrent(module.GL.registerContext(gl, { majorVersion: 2, minorVersion: 0 }));
  module._omni_render_view_open_readable();

  // Hover and selection outlines are elements over the canvas.
  const overlay = document.createElement("div");
  overlay.setAttribute("aria-hidden", "true");
  overlay.style.cssText = "position:absolute;inset:0;overflow:hidden;pointer-events:none";
  canvas.insertAdjacentElement("afterend", overlay);

  const listeners = new Set<(address: string | null) => void>();
  const viewListeners = new Set<() => void>();
  let loaded = false;
  let dirty = true;
  // Whether the page shown has not been drawn yet, so that its view is not fitted to it.
  let unfitted = false;
  let frame = 0;
  let destroyed = false;
  let pointer: [number, number] | null = null;
  let keys = { ctrl: false, meta: false };
  // The canvas's size in CSS pixels. The resize observer keeps it, so that a
  // frame does not read it from the page, which would lay the page out again
  // after whatever React changed in it since the last frame.
  let cssWidth = canvas.clientWidth;
  let cssHeight = canvas.clientHeight;
  // The system the module resolves pages against, so that showing a page again
  // with the same system does not lay out the page that is loaded for nothing.
  let systemSent: unknown;
  // The stage the canvas was moved into while a prototype plays, and where it
  // sat before: the canvas goes back there when the session ends.
  let playing: { parent: Node; next: Node | null; style: string } | null = null;
  // Whether the stage has to be drawn at its size (the first frame, a resize).
  let playResized = false;
  const playListeners = new Set<() => void>();

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
    if (playing) return;
    unfitted = false;
    const ratio = window.devicePixelRatio || 1;
    const width = Math.max(1, Math.round(cssWidth * ratio));
    const height = Math.max(1, Math.round(cssHeight * ratio));
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
    const drew = module._omni_render_scene_draw(canvas.width, canvas.height, ratio);
    if (!drew) throw failure();
    // The renderer says (2) when a board was drawn from a picture older or smaller than the view calls for: it makes the sharper one in the next frame.
    if (drew === 2) requestFrame();
    drawOverlay();
    // Every frame drawn is a view that may have changed (zoom, pan, resize,
    // another page), and the comment pins sit on the new one.
    for (const listener of Array.from(viewListeners)) listener();
  }

  // Frames are asked for, not looped: nothing is scheduled while the view is idle.
  let scheduled = false;
  function requestFrame() {
    dirty = true;
    if (scheduled || destroyed) return;
    scheduled = true;
    frame = requestAnimationFrame(loop);
  }

  /** Draws the stage, which asks for another frame while a transition plays (`play_tick` says 2) or while a wait runs (3). */
  function playFrame(now: number) {
    const ratio = window.devicePixelRatio || 1;
    const width = Math.max(1, Math.round(cssWidth * ratio));
    const height = Math.max(1, Math.round(cssHeight * ratio));
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
      playResized = true;
    }
    if (playResized) {
      playResized = false;
      if (!module._omni_render_play_draw(width, height, ratio)) throw failure();
    }
    // 2: a transition plays; 3: nothing to draw yet, but a wait has to be seen to run out.
    const ticked = module._omni_render_play_tick(now);
    if (ticked === 2 || ticked === 3) requestFrame();
  }

  function playChanged() {
    requestFrame();
    for (const listener of Array.from(playListeners)) listener();
  }

  function loop(now: number) {
    scheduled = false;
    if (destroyed) return;
    if (dirty && loaded) {
      dirty = false;
      if (playing) playFrame(now);
      else drawFrame();
    }
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
    const unit = [1, 16, cssHeight][event.deltaMode] ?? 1;
    if (playing) {
      // The wheel scrolls what is under the pointer; the browser's own scrolling stays out of play.
      event.preventDefault();
      if (module._omni_render_play_wheel(x, y, event.deltaX * unit, event.deltaY * unit)) playChanged();
      return;
    }
    // Ctrl + wheel is also how browsers report a trackpad pinch.
    if (module._omni_render_view_wheel(event.deltaX * unit, event.deltaY * unit, x, y, event.ctrlKey, event.metaKey)) {
      event.preventDefault();
      requestFrame();
    }
  };
  const playPointer = (kind: number, event: PointerEvent) => {
    const [x, y] = position(event);
    if (module._omni_render_play_pointer(kind, x, y)) playChanged();
  };
  const onPointerDown = (event: PointerEvent) => {
    if (!loaded) return;
    if (playing) {
      playPointer(0, event);
      return;
    }
    const [x, y] = position(event);
    canvas.setPointerCapture?.(event.pointerId);
    ignoreClick = false;
    const middle = event.button === 1;
    // The middle button would otherwise start the browser's autoscroll.
    if (middle) event.preventDefault();
    if (module._omni_render_view_pointer_down(x, y, middle)) ignoreClick = true;
  };
  const onPointerMove = (event: PointerEvent) => {
    if (!loaded) return;
    if (playing) {
      playPointer(1, event);
      return;
    }
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
  const onPointerUp = (event: PointerEvent) => {
    if (playing) {
      if (event.type === "pointerup") playPointer(2, event);
      else playPointer(3, event);
      return;
    }
    if (loaded) module._omni_render_view_pointer_up();
  };
  const onPointerLeave = (event: PointerEvent) => {
    if (!loaded) return;
    if (playing) {
      playPointer(3, event);
      return;
    }
    pointer = null;
    // A point that is nowhere, so that hover clears.
    module._omni_render_select_pointer_move(NaN, NaN, false, false);
    takeEvents();
  };
  const onClick = (event: MouseEvent) => {
    if (!loaded || playing) return;
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
    if (!loaded || playing) return;
    const [x, y] = position(event);
    module._omni_render_select_double_click(x, y);
    pointer = [x, y];
    hover();
    takeEvents();
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (!loaded || playing) return;
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
    if (!loaded || playing) return;
    keys = { ctrl: event.ctrlKey, meta: event.metaKey };
    if (event.code === "Space") module._omni_render_view_space(false);
  };
  const onBlur = () => {
    if (loaded && !playing) module._omni_render_view_space(false);
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
  const resizing = new ResizeObserver(() => {
    cssWidth = canvas.clientWidth;
    cssHeight = canvas.clientHeight;
    requestFrame();
  });
  resizing.observe(canvas);
  requestFrame();

  return {
    addImage(attachment, bytes) {
      const added = withText(attachment, (name, nameLength) =>
        withBytes(bytes, (data, dataLength) => module._omni_render_add_image(name, nameLength, data, dataLength)),
      );
      if (!added) throw failure();
    },
    addPreview(board, bytes) {
      const added = withText(board, (name, nameLength) =>
        withBytes(bytes, (data, dataLength) => module._omni_render_add_preview(name, nameLength, data, dataLength)),
      );
      if (!added) throw failure();
    },
    addFont(family, bytes) {
      const added = withText(family, (name, nameLength) =>
        withBytes(bytes, (data, dataLength) => module._omni_render_add_font(name, nameLength, data, dataLength)),
      );
      if (!added) throw failure();
    },
    show(page, system, board) {
      if (!withText(board ?? "", (pointer, length) => module._omni_render_view_open_on(pointer, length))) throw failure();
      if (system !== systemSent) {
        const ok = withText(JSON.stringify(system), (pointer, length) => module._omni_render_scene_set_system(pointer, length));
        if (!ok) throw failure();
        systemSent = system;
      }
      if (!withText(JSON.stringify(page), (pointer, length) => module._omni_render_scene_load(pointer, length))) {
        throw failure();
      }
      loaded = true;
      unfitted = true;
      requestFrame();
    },
    usePreviews(boards) {
      if (!loaded) return;
      if (!withText(JSON.stringify(boards), (pointer, length) => module._omni_render_scene_use_previews(pointer, length))) {
        throw failure();
      }
    },
    updateBoards(boards) {
      if (!loaded) return false;
      const queued = withText(JSON.stringify(boards), (pointer, length) => module._omni_render_scene_update_boards(pointer, length));
      if (queued < 0) throw failure();
      return queued > 0;
    },
    stepBoards() {
      if (!loaded) return false;
      const done = module._omni_render_scene_update_step();
      // The bit 2 says a board was put in place: the next frame draws it.
      if (done !== 0) requestFrame();
      if (done < 0) throw failure();
      return (done & 1) !== 0;
    },
    showTags(tags) {
      if (!withText(tags, (pointer, length) => module._omni_render_scene_load_tags(pointer, length))) throw failure();
      loaded = true;
      dirty = false;
      drawFrame();
    },
    visibleBoards() {
      if (!loaded || playing) return [];
      // A page shown a moment ago has no view yet: draw it, so that the view is fitted to it.
      // A page that has been drawn is not drawn here: a frame drawn between the
      // animation frames would leave the next one with nothing to draw, and
      // the frames would come at uneven times.
      if (unfitted) {
        dirty = false;
        drawFrame();
      }
      return readJson<string[]>(module._omni_render_visible_boards());
    },
    layers() {
      return loaded ? readJson<Layer[]>(module._omni_render_scene_layers()) : [];
    },
    fontsStoodIn() {
      return loaded ? readJson<FontStandIn[]>(module._omni_render_scene_fonts()) : [];
    },
    layersOf(address) {
      if (!loaded) return null;
      const text = withText(address, (pointer, length) => module._omni_render_scene_layers_of(pointer, length));
      return text ? readJson<Layer>(text) : null;
    },
    boardOf(address) {
      if (!loaded) return null;
      const text = withText(address, (pointer, length) => module._omni_render_scene_board_of(pointer, length));
      return text ? module.UTF8ToString(text) : null;
    },
    select(address, reveal = false) {
      if (!loaded) return;
      withText(address ?? "", (pointer, length) => module._omni_render_select_node(pointer, length));
      // The caller knows about this selection, so it is not announced.
      module._omni_render_take_events();
      if (reveal) {
        module._omni_render_view_reveal_selected();
        requestFrame();
      }
      drawOverlay();
    },
    setMode(choice) {
      if (!loaded) return;
      const text = typeof choice === "string" ? choice : JSON.stringify(choice);
      if (!withText(text, (pointer, length) => module._omni_render_scene_set_mode(pointer, length))) throw failure();
      requestFrame();
    },
    inspect(address) {
      if (!loaded) return null;
      const text = withText(address, (pointer, length) => module._omni_render_inspect(pointer, length));
      return text ? readJson<Inspection>(text) : null;
    },
    boxOf(address) {
      if (!loaded) return null;
      const text = withText(address, (pointer, length) => module._omni_render_node_box(pointer, length));
      return text ? readJson<{ x: number; y: number; width: number; height: number; name: string }>(text) : null;
    },
    onSelect(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    onView(listener) {
      viewListeners.add(listener);
      return () => {
        viewListeners.delete(listener);
      };
    },
    play: {
      start(board, stage) {
        if (!loaded) throw new Error("no scene is loaded");
        if (!withText(board, (pointer, length) => module._omni_render_play_start(pointer, length))) throw failure();
        if (!playing) {
          playing = { parent: canvas.parentNode as Node, next: canvas.nextSibling, style: canvas.style.cssText };
          overlay.replaceChildren();
          if (stage) {
            canvas.style.cssText = "position:absolute;inset:0;width:100%;height:100%;touch-action:none";
            stage.append(canvas);
          }
          pointer = null;
        }
        cssWidth = canvas.clientWidth;
        cssHeight = canvas.clientHeight;
        playResized = true;
        playChanged();
      },
      stop() {
        module._omni_render_play_stop();
        if (!playing) return;
        const { parent, next, style } = playing;
        playing = null;
        canvas.style.cssText = style;
        // The canvas goes back where it sat, before the element that followed it.
        parent.insertBefore(canvas, next && next.parentNode === parent ? next : null);
        cssWidth = canvas.clientWidth;
        cssHeight = canvas.clientHeight;
        requestFrame();
      },
      pointer(kind, x, y) {
        const code = typeof kind === "number" ? kind : ["down", "move", "up", "leave"].indexOf(kind);
        const changed = module._omni_render_play_pointer(code, x, y);
        if (changed) playChanged();
        return changed;
      },
      key(event) {
        const fired = withText(JSON.stringify(event), (pointer, length) => module._omni_render_play_key(pointer, length));
        if (fired) playChanged();
        return fired;
      },
      gamepad(button, down) {
        const fired = module._omni_render_play_gamepad(button, down);
        if (fired) playChanged();
        return fired;
      },
      back() {
        const moved = module._omni_render_play_back();
        if (moved) playChanged();
        return moved;
      },
      wheel(x, y, dx, dy) {
        const moved = module._omni_render_play_wheel(x, y, dx, dy);
        if (moved) playChanged();
        return moved;
      },
      setDevice(name) {
        if (!withText(name, (pointer, length) => module._omni_render_play_device(pointer, length))) throw failure();
        playChanged();
      },
      setScale(scale) {
        if (!withText(scale, (pointer, length) => module._omni_render_play_scale(pointer, length))) throw failure();
        playChanged();
      },
      screen() {
        const text = module._omni_render_play_screen();
        if (!text) throw failure();
        return readJson<PlayScreen>(text);
      },
      go(board) {
        const moved = withText(board, (pointer, length) => module._omni_render_play_go(pointer, length));
        if (moved) playChanged();
        return moved;
      },
      restart(board) {
        if (!withText(board, (pointer, length) => module._omni_render_play_restart(pointer, length))) throw failure();
        playChanged();
      },
      state() {
        const text = module._omni_render_play_state();
        if (!text) throw failure();
        return readJson<PlayState>(text);
      },
      onChange(listener) {
        playListeners.add(listener);
        return () => {
          playListeners.delete(listener);
        };
      },
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
      viewListeners.clear();
      playListeners.clear();
    },
  };
}
