// US-042: the doc viewer's frame, in a real browser. Chrome, installed on the
// machine, is driven through Playwright (`channel: "chrome"`; `CHROME` names
// another executable), headless, as the renderer gates of render-sandbox/ do.
//
// A local server stands in for the app and the API. It serves a page that
// holds the frame as the doc page does, `<iframe sandbox="allow-scripts"
// srcdoc="...">` with the document of `viewerDocument({ workspaceId, origin })`
// (src/viewer/document.ts), and it sets the session cookie on that page as the
// API does at login (`omni_session`, HttpOnly, SameSite=Lax, Path=/). It
// records every request with the cookies it carried.
//
// The frame and the page talk only through postMessage. The messages are:
//
// - from the frame to the page (`parent.postMessage(message, "*")`):
//   - `{ type: "viewer.ready" }` once the viewer's script runs, so that the page
//     knows the frame can be shown something;
//   - `{ type: "viewer.select", node }` when the reviewer clicks a block: the
//     node ID of the innermost block under the pointer. A click on inline
//     markup (a `<strong>` in a paragraph) selects the text block it is in.
// - from the page to the frame (`frame.contentWindow.postMessage(message,
//   "*")`): `{ type: "viewer.show", html, pins, selected, claims }`, sent after
//   each `viewer.ready` and whenever any of it changes:
//   - `html`: the doc's HTML as the API prints it, with `data-omni-id` on every
//     block-level element (US-109), which the frame shows with the default
//     stylesheet;
//   - `pins`: `[{ node, count }]`, the blocks that have comments and how many,
//     which the frame marks with a pin beside the block that shows the count;
//   - `selected`: the selected block's node ID, or `null`; the frame shows it
//     as selected;
//   - `claims`: the doc's live claims, as the API lists them (the page shows
//     them too);
//   - `reveal` (US-112): a block's node ID, which the frame scrolls into view.
//
// What the frame does, tested here:
//
// - Isolation: inside the frame, `parent.document` throws, and no request that
//   the frame makes carries the session. The policy lets nothing but images of
//   the workspace's attachments reach the network: a fetch, an XHR, a beacon,
//   a stylesheet, a font or an image from anywhere else never reaches the
//   server.
// - US-109: a doc holds any HTML, CSS and inline JS. Its own scripts and
//   event handlers run inside the frame, which keeps `sandbox="allow-scripts"`
//   without `allow-same-origin`: they cannot read the page's document, and
//   the policy blocks every request they make (a `fetch` to
//   https://example.com fails with a policy violation and nothing reaches the
//   server).
// - US-109: a doc without styles of its own is readable with OmniState's
//   default stylesheet (a sans-serif body with roomy lines and a measure, and
//   table cells with padding and borders), and a doc's own `<style>` overrides
//   it, even with plain element selectors.
// - Mermaid figures (`<figure><pre class="mermaid">`) are drawn by the viewer as
//   an SVG in the figure, in place of the Mermaid text.
// - A frame without a size (0 wide and 0 tall, as it is in a browser tab that
//   loads in the background until the tab is shown) shows the doc's HTML, but
//   the doc's scripts run and its Mermaid figures are drawn only once the
//   frame has a size, because nothing can be measured before that.
// - Hovering a block highlights it, and moving on takes the highlight away;
//   clicking a block selects it (the frame tells the page), a selected block
//   looks selected, and pins show beside the blocks that have comments.
//
// A highlight, a selection mark or a pin is anything that changes the pixels
// of the block's row (the frame's width across the block's height): an
// outline, a background, a badge.
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { chromium, type Browser, type Frame, type Page } from "playwright-core";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test } from "vitest";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const OTHER_WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000b";
const ATTACHMENT_ID = "0192f1c4-5b6a-7d8e-9f01-23456789abcd";
const SESSION_COOKIE = "omni_session";
const SESSION_SECRET = "harness-session-0123456789abcdef0123456789abcdef";
/** Where the frame sits in the page; the mouse rests outside it at (5, 5). */
const FRAME_LEFT = 40;
const FRAME_TOP = 40;
const FRAME_WIDTH = 900;
const FRAME_HEIGHT = 1200;
const PATIENCE = 15_000;

/** The doc of docs/formats/html-docs.md: every block type, an SVG and a Mermaid figure. */
const FULL_DOC = `<section data-omni-id="a0000001">
<h1 data-omni-id="a0000002">Order service</h1>
<p data-omni-id="a0000003">Plain, <strong>strong</strong>, <em>emphasis</em>, <code>code()</code> and <a href="https://example.com/docs">a link</a>.<br>After the line break.</p>
<blockquote data-omni-id="a0000004">Measure twice, cut once.</blockquote>
<hr data-omni-id="a0000005">
<ul data-omni-id="a0000006">
<li data-omni-id="a0000007">First item</li>
<li data-omni-id="a0000008">Second item</li>
</ul>
<ol data-omni-id="a0000009">
<li data-omni-id="a0000010">Step one</li>
</ol>
<table data-omni-id="a0000011">
<tr data-omni-id="a0000012"><th data-omni-id="a0000013">Field</th><th data-omni-id="a0000014">Type</th></tr>
<tr data-omni-id="a0000015"><td data-omni-id="a0000016">id</td><td data-omni-id="a0000017">uuid</td></tr>
</table>
<figure data-omni-id="a0000018"><svg viewBox="0 0 100 50"><rect x="1" y="1" width="50" height="20" fill="#336699"/><circle cx="70" cy="25" r="10"/></svg><figcaption>A box and a circle</figcaption></figure>
<figure data-omni-id="a0000019"><pre class="mermaid">graph TD;
  Alpha--&gt;Beta</pre><figcaption>The flow</figcaption></figure>
<aside data-omni-id="a0000020" data-callout="note">A note.</aside>
<aside data-omni-id="a0000021" data-callout="warning">Mind the gap.</aside>
<aside data-omni-id="a0000022" data-callout="decision">Orders are never deleted.</aside>
<pre data-omni-id="a0000023"><code>fn main() {
    println!("hi");
}</code></pre>
<img data-omni-id="a0000024" src="attachment:${ATTACHMENT_ID}" alt="A screenshot">
<p data-omni-id="a0000025" data-block="requirement">FR-1: The system must keep every version.</p>
<article data-omni-id="a0000026" data-block="user-story">
<h3 data-omni-id="a0000027">Reset a password</h3>
<p data-omni-id="a0000028">As a user, I want to reset my password so that I can get back in.</p>
<ul data-omni-id="a0000029"><li data-omni-id="a0000030">The link works once</li></ul>
</article>
</section>`;

/** A short doc whose blocks all fit in the frame; no digit appears in its text. */
const SHORT_DOC = `<section data-omni-id="s0000001">
<h1 data-omni-id="h0000001">Checkout</h1>
<p data-omni-id="p0000001">Orders ship in <strong>one</strong> day.</p>
<p data-omni-id="p0000002">Refunds take a week.</p>
<ul data-omni-id="t0000001"><li data-omni-id="k0000001">Cards</li><li data-omni-id="k0000002">Invoices</li></ul>
<p data-omni-id="p0000003">Returns are free.</p>
<p data-omni-id="p0000004">Gift wrap is extra.</p>
<p data-omni-id="p0000005">Taxes are shown at the end.</p>
</section>`;

/** A doc several frames tall: sixty paragraphs, p0000101 to p0000160. */
const TALL_DOC = `<section data-omni-id="s0000002">
${Array.from({ length: 60 }, (_, i) => `<p data-omni-id="p${String(101 + i).padStart(7, "0")}">Paragraph ${101 + i} of the tall doc, long enough to wrap onto a second line in the frame so that the doc is several frames tall.</p>`).join("\n")}
</section>`;

/** A one-pixel PNG. */
const PIXEL = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

type ViewerModule = {
  viewerDocument(options: { workspaceId: string; origin: string }): string;
};

/** Imports src/viewer/document.ts, failing the test (not the suite) when it is missing. */
async function loadViewer(): Promise<ViewerModule> {
  const specifier = "./viewer/document";
  try {
    return (await import(/* @vite-ignore */ specifier)) as ViewerModule;
  } catch (error) {
    expect.fail(`src/viewer/document.ts cannot be imported: ${String(error)}`);
  }
}

type Seen = { path: string; cookie: string };

let server: Server;
let origin = "";
let browser: Browser | null = null;
let launchError: unknown = null;
let frameDocument = "";
let seen: Seen[] = [];
let page: Page | null = null;

function escapeAttribute(text: string) {
  return text.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function harnessPage() {
  return `<!doctype html>
<html><head><meta charset="utf-8"><title>Viewer harness</title></head>
<body style="margin:0;background:#fff">
<script>
window.got = [];
window.addEventListener("message", (event) => {
  const frame = document.getElementById("viewer");
  if (frame && event.source === frame.contentWindow) window.got.push(event.data);
});
</script>
<iframe id="viewer" title="Doc" sandbox="allow-scripts"
  style="position:absolute;left:${FRAME_LEFT}px;top:${FRAME_TOP}px;width:${FRAME_WIDTH}px;height:${FRAME_HEIGHT}px;border:0"
  srcdoc="${escapeAttribute(frameDocument)}"></iframe>
</body></html>`;
}

function serve(request: IncomingMessage, response: ServerResponse) {
  const path = new URL(request.url ?? "/", "http://harness").pathname;
  seen.push({ path, cookie: request.headers.cookie ?? "" });
  if (path === "/") {
    response.writeHead(200, {
      "content-type": "text/html; charset=utf-8",
      "set-cookie": `${SESSION_COOKIE}=${SESSION_SECRET}; HttpOnly; SameSite=Lax; Path=/`,
    });
    response.end(harnessPage());
    return;
  }
  if (path.includes("/attachments/") || path.endsWith(".png")) {
    response.writeHead(200, { "content-type": "image/png", "access-control-allow-origin": "*" });
    response.end(PIXEL);
    return;
  }
  response.writeHead(200, { "content-type": "application/json", "access-control-allow-origin": "*" });
  response.end("{}");
}

/** The browser, failing the test when Chrome could not be started. */
function chrome(): Browser {
  if (!browser) {
    expect.fail(`Chrome could not be started through Playwright (set CHROME to its path): ${String(launchError)}`);
  }
  return browser;
}

/** Opens the harness with the viewer's document and waits for the viewer to say it is ready. */
async function openViewer(): Promise<{ page: Page; frame: Frame }> {
  const { viewerDocument } = await loadViewer();
  frameDocument = viewerDocument({ workspaceId: WORKSPACE_ID, origin });
  page = await chrome().newPage({ viewport: { width: FRAME_LEFT + FRAME_WIDTH + 40, height: FRAME_TOP + FRAME_HEIGHT + 40 } });
  await page.goto(`${origin}/`);
  await page.mouse.move(5, 5);
  await page.waitForFunction(
    () => (window as unknown as { got: { type?: string }[] }).got.some((m) => m?.type === "viewer.ready"),
    undefined,
    { timeout: PATIENCE },
  );
  const frame = page.frames().find((candidate) => candidate !== page!.mainFrame());
  expect(frame, "the viewer frame").toBeTruthy();
  return { page, frame: frame as Frame };
}

type Shown = {
  html: string;
  pins?: { node: string; count: number }[];
  selected?: string | null;
  reveal?: string;
};

/** Posts `viewer.show` to the frame. */
async function post(target: { page: Page }, message: Shown) {
  await target.page.evaluate((data) => {
    const frame = document.getElementById("viewer") as HTMLIFrameElement;
    frame.contentWindow!.postMessage(data, "*");
  }, { type: "viewer.show", pins: [], selected: null, claims: [], ...message });
}

/** Posts `viewer.show` to the frame and waits until it shows the block `waitFor`. */
async function show(target: { page: Page; frame: Frame }, message: Shown, waitFor: string) {
  await post(target, message);
  await target.frame.waitForSelector(`[data-omni-id="${waitFor}"]`, { timeout: PATIENCE });
}

/**
 * Takes the frame's size away (0 wide and 0 tall) or gives it back, and waits until the frame
 * knows. A frame in a browser tab that loads in the background is like this until the tab is
 * shown.
 */
async function sizeless(target: { page: Page; frame: Frame }, on: boolean) {
  await target.page.evaluate(
    ({ none, width, height }) => {
      const frame = document.getElementById("viewer")!;
      frame.style.width = none ? "0" : `${width}px`;
      frame.style.height = none ? "0" : `${height}px`;
    },
    { none: on, width: FRAME_WIDTH, height: FRAME_HEIGHT },
  );
  await expect
    .poll(() => target.frame.evaluate(() => window.innerWidth === 0), { timeout: PATIENCE })
    .toBe(on);
}

/** The messages the frame posted to the page. */
async function got(target: { page: Page }): Promise<{ type?: string; node?: string }[]> {
  return target.page.evaluate(() => (window as unknown as { got: { type?: string; node?: string }[] }).got);
}

/** A screenshot of the frame's row of the block: the frame's width across the block's height, with a margin. */
async function row(target: { page: Page; frame: Frame }, node: string) {
  const box = await target.frame.locator(`[data-omni-id="${node}"]`).boundingBox();
  expect(box, `the block ${node} is shown`).toBeTruthy();
  const top = Math.max(FRAME_TOP, Math.floor(box!.y) - 4);
  const bottom = Math.min(FRAME_TOP + FRAME_HEIGHT, Math.ceil(box!.y + box!.height) + 4);
  return target.page.screenshot({
    clip: { x: FRAME_LEFT, y: top, width: FRAME_WIDTH, height: bottom - top },
  });
}

/** A screenshot of the block's own box. */
async function boxOf(target: { page: Page; frame: Frame }, node: string) {
  return target.frame.locator(`[data-omni-id="${node}"]`).screenshot();
}

/** Moves the mouse, in steps as a hand does, to the middle of the block's first line. */
async function pointAt(target: { page: Page; frame: Frame }, node: string) {
  const box = await target.frame.locator(`[data-omni-id="${node}"]`).boundingBox();
  expect(box, `the block ${node} is shown`).toBeTruthy();
  await target.page.mouse.move(box!.x + Math.min(box!.width / 2, 40), box!.y + Math.min(box!.height / 2, 8), { steps: 8 });
}

/** Waits until `take` gives a picture that is (or is not) `reference`. */
async function settle(take: () => Promise<Buffer>, reference: Buffer, same: boolean, what: string) {
  const deadline = Date.now() + 5_000;
  let latest = await take();
  while (latest.equals(reference) !== same && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 100));
    latest = await take();
  }
  expect(latest.equals(reference), what).toBe(same);
}

beforeAll(async () => {
  server = createServer(serve);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  try {
    browser = await chromium.launch({
      channel: "chrome",
      executablePath: process.env.CHROME || undefined,
      headless: true,
    });
  } catch (error) {
    launchError = error;
  }
}, 60_000);

afterAll(async () => {
  await browser?.close();
  await new Promise((resolve) => server.close(resolve));
});

beforeEach(() => {
  seen = [];
});

afterEach(async () => {
  await page?.close();
  page = null;
});

describe("the viewer frame in Chrome", { timeout: 60_000 }, () => {
  test("no request from the frame carries the session, and only the workspace's attachments reach the network", async () => {
    const target = await openViewer();
    // The cookie is there: the page itself sends it.
    await target.page.evaluate(() => fetch("/control", { credentials: "include" }));
    expect(seen.find((s) => s.path === "/control")?.cookie).toContain(`${SESSION_COOKIE}=${SESSION_SECRET}`);

    await show(target, { html: FULL_DOC }, "a0000024");
    await target.frame.evaluate(
      async ({ origin, workspace, other, attachment }) => {
        const api = `${origin}/api/v1/w/${workspace}`;
        const settle = (work: Promise<unknown>) =>
          Promise.race([work.catch(() => null), new Promise((resolve) => setTimeout(resolve, 2_000))]);
        const image = (url: string) =>
          new Promise((resolve) => {
            const img = new Image();
            img.onload = img.onerror = resolve;
            img.src = url;
          });
        const xhr = (url: string) =>
          new Promise((resolve) => {
            const request = new XMLHttpRequest();
            request.open("GET", url);
            request.withCredentials = true;
            request.onloadend = resolve;
            try {
              request.send();
            } catch {
              resolve(null);
            }
          });
        const stylesheet = (url: string) =>
          new Promise((resolve) => {
            const link = document.createElement("link");
            link.rel = "stylesheet";
            link.href = url;
            link.onload = link.onerror = resolve;
            document.head.append(link);
          });
        const font = (url: string) => new FontFace("Probe", `url(${url})`).load();
        try {
          navigator.sendBeacon(`${origin}/api/beacon`, "x");
        } catch {
          // A blocked beacon may throw.
        }
        await Promise.all(
          [
            fetch(`${api}/whoami`, { credentials: "include" }),
            fetch(`${api}/whoami`, { credentials: "include", mode: "no-cors" }),
            fetch(`${origin}/elsewhere/fetch`),
            xhr(`${api}/whoami?via=xhr`),
            image(`${api}/attachments/${attachment}?via=image`),
            image(`${origin}/elsewhere/image.png`),
            image(`${origin}/api/v1/w/${other}/attachments/${attachment}`),
            stylesheet(`${origin}/elsewhere/style.css`),
            font(`${origin}/elsewhere/font.woff2`),
          ].map(settle),
        );
      },
      { origin, workspace: WORKSPACE_ID, other: OTHER_WORKSPACE_ID, attachment: ATTACHMENT_ID },
    );
    await new Promise((resolve) => setTimeout(resolve, 500));

    const fromFrame = seen.filter((s) => s.path !== "/" && s.path !== "/control" && s.path !== "/favicon.ico");
    expect(fromFrame.map((s) => s.path), "the workspace's attachment image reaches the server").toContain(
      `/api/v1/w/${WORKSPACE_ID}/attachments/${ATTACHMENT_ID}`,
    );
    for (const request of fromFrame) {
      expect(request.cookie, `${request.path} carries no session`).not.toContain(SESSION_COOKIE);
      expect(
        request.path.startsWith(`/api/v1/w/${WORKSPACE_ID}/attachments/`),
        `${request.path} was fetched, but only the workspace's attachments may be`,
      ).toBe(true);
    }
  });

  test("a doc's own scripts and handlers run in the frame", async () => {
    const target = await openViewer();
    const html = `<section data-omni-id="s0000001">
<p data-omni-id="p0000001" id="by-script">Waiting.</p>
<p data-omni-id="p0000002" id="by-handler" onclick="this.textContent = 'Clicked.'">Click here.</p>
<script>
document.getElementById("by-script").textContent = "Ran.";
</script>
</section>`;
    await show(target, { html }, "p0000002");
    const text = (id: string) => target.frame.evaluate((which) => document.getElementById(which)?.textContent ?? "", id);
    await expect.poll(() => text("by-script"), { timeout: PATIENCE }).toBe("Ran.");
    await target.frame.click('[data-omni-id="p0000002"]', { force: true });
    await expect.poll(() => text("by-handler"), { timeout: PATIENCE }).toBe("Clicked.");
  });

  test("a doc's scripts wait until the frame has a size", async () => {
    const target = await openViewer();
    await sizeless(target, true);
    const html = `<section data-omni-id="s0000001">
<p data-omni-id="p0000001" id="measured">Waiting.</p>
<script>
document.getElementById("measured").textContent = "Measured " + window.innerWidth + ".";
</script>
</section>`;
    await post(target, { html });
    await target.frame.waitForSelector('[data-omni-id="p0000001"]', { state: "attached", timeout: PATIENCE });
    const text = () => target.frame.evaluate(() => document.getElementById("measured")?.textContent ?? "");
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(await text(), "the script has not run in a frame without a size").toBe("Waiting.");
    await sizeless(target, false);
    await expect.poll(text, { timeout: PATIENCE }).toBe(`Measured ${FRAME_WIDTH}.`);
  });

  test("a doc without styles is readable with the default stylesheet, and a doc's own style overrides it", async () => {
    const target = await openViewer();
    const plain = `<section data-omni-id="s0000001">
<h1 data-omni-id="h0000001">Checkout</h1>
<p data-omni-id="p0000001">Orders ship in one day, and refunds take a week.</p>
<div data-omni-id="d0000001" class="box"><p data-omni-id="p0000002">Inside a div.</p></div>
<table data-omni-id="t0000001"><tbody><tr data-omni-id="r0000001"><td data-omni-id="c0000001">Cell</td></tr></tbody></table>
</section>`;
    await show(target, { html: plain }, "c0000001");
    const looks = () =>
      target.frame.evaluate(() => {
        const style = (id: string) => getComputedStyle(document.querySelector(`[data-omni-id="${id}"]`)!);
        const paragraph = style("p0000001");
        const cell = style("c0000001");
        const heading = style("h0000001");
        return {
          font: paragraph.fontFamily,
          lineHeight: parseFloat(paragraph.lineHeight) / parseFloat(paragraph.fontSize),
          width: document.querySelector('[data-omni-id="p0000001"]')!.getBoundingClientRect().width,
          cellPadding: parseFloat(cell.paddingLeft),
          cellBorder: parseFloat(cell.borderLeftWidth),
          headingSize: heading.fontSize,
          headingColor: heading.color,
        };
      });
    const readable = await looks();
    expect(readable.font, "the body text is not the browser's default serif").not.toMatch(/times|^serif$/i);
    expect(readable.lineHeight, "lines have room").toBeGreaterThanOrEqual(1.4);
    expect(readable.width, "text keeps a readable measure").toBeLessThanOrEqual(800);
    expect(readable.cellPadding, "table cells have padding").toBeGreaterThan(0);
    expect(readable.cellBorder, "table cells have a border").toBeGreaterThan(0);

    const styled = plain.replace(
      '<h1 data-omni-id="h0000001">',
      `<style>h1 { font-size: 11px; color: rgb(1, 2, 3); } p { font-family: serif; line-height: 1; } td { padding: 0; border: 0 none; }</style>
<h1 data-omni-id="h0000001">`,
    );
    await show(target, { html: styled }, "c0000001");
    await expect.poll(async () => (await looks()).headingSize, { timeout: PATIENCE }).toBe("11px");
    const own = await looks();
    expect(own.headingColor).toBe("rgb(1, 2, 3)");
    expect(own.font).toBe("serif");
    expect(own.lineHeight).toBeCloseTo(1, 1);
    expect(own.cellPadding).toBe(0);
    expect(own.cellBorder).toBe(0);
  });

  test("Mermaid figures are drawn by the viewer", async () => {
    const target = await openViewer();
    await show(target, { html: FULL_DOC }, "a0000019");
    await target.frame.waitForFunction(
      () => {
        const figure = document.querySelector('[data-omni-id="a0000019"]');
        const svg = figure?.querySelector("svg");
        return Boolean(svg && /Alpha/.test(svg.textContent ?? "") && /Beta/.test(svg.textContent ?? ""));
      },
      undefined,
      { timeout: PATIENCE },
    );
    const figure = await target.frame.evaluate(() => {
      const element = document.querySelector('[data-omni-id="a0000019"]') as HTMLElement;
      return { text: element.innerText, caption: element.querySelector("figcaption")?.textContent ?? "" };
    });
    expect(figure.text, "the Mermaid source is drawn, not shown as text").not.toMatch(/graph TD/);
    expect(figure.caption).toBe("The flow");
  });

  test("Mermaid figures are drawn once the frame has a size", async () => {
    const target = await openViewer();
    await sizeless(target, true);
    await post(target, { html: FULL_DOC });
    await target.frame.waitForSelector('[data-omni-id="a0000019"]', { state: "attached", timeout: PATIENCE });
    await new Promise((resolve) => setTimeout(resolve, 1_000));
    const waits = await target.frame.evaluate(() => {
      const figure = document.querySelector('[data-omni-id="a0000019"]');
      return Boolean(figure?.querySelector("pre.mermaid")) && !figure?.querySelector("svg");
    });
    expect(waits, "the figure is not drawn in a frame without a size").toBe(true);
    await sizeless(target, false);
    // The boxes of Alpha and Beta, each wider than its text is at any font size a reader could have.
    const boxes = () =>
      target.frame.evaluate(() =>
        Array.from(document.querySelectorAll('[data-omni-id="a0000019"] svg .node'), (node) =>
          Math.round(node.getBoundingClientRect().width),
        ),
      );
    await expect.poll(async () => (await boxes()).length, { timeout: PATIENCE }).toBe(2);
    for (const width of await boxes()) expect(width, "a box of the figure has room for its text").toBeGreaterThan(30);
  });

  test("hovering highlights the block under the pointer, and only while it is there", async () => {
    const target = await openViewer();
    await show(target, { html: SHORT_DOC }, "p0000005");
    const first = await row(target, "p0000001");
    const third = await row(target, "p0000003");
    const far = await boxOf(target, "p0000005");

    await pointAt(target, "p0000001");
    await settle(() => row(target, "p0000001"), first, false, "the hovered block is highlighted");
    await settle(() => boxOf(target, "p0000005"), far, true, "a block far from the pointer is not");

    await pointAt(target, "p0000003");
    await settle(() => row(target, "p0000003"), third, false, "the next hovered block is highlighted");
    await settle(() => row(target, "p0000001"), first, true, "the block the pointer left is not any more");

    await target.page.mouse.move(5, 5, { steps: 8 });
    await settle(() => row(target, "p0000003"), third, true, "with the pointer outside, no block is");
  });

  test("clicking a block selects the innermost block under the pointer", async () => {
    const target = await openViewer();
    await show(target, { html: SHORT_DOC }, "k0000002");
    const lastSelect = async () =>
      (await got(target)).filter((m) => m?.type === "viewer.select").map((m) => m.node).pop();

    await target.frame.click('[data-omni-id="p0000002"]', { force: true });
    await expect.poll(lastSelect, { timeout: PATIENCE }).toBe("p0000002");

    await target.frame.click('[data-omni-id="p0000001"] strong', { force: true });
    await expect.poll(lastSelect, { timeout: PATIENCE }).toBe("p0000001");

    await target.frame.click('[data-omni-id="k0000002"]', { force: true });
    await expect.poll(lastSelect, { timeout: PATIENCE }).toBe("k0000002");

    await target.frame.click('[data-omni-id="h0000001"]', { force: true });
    await expect.poll(lastSelect, { timeout: PATIENCE }).toBe("h0000001");
  });

  test("the selected block looks selected", async () => {
    const target = await openViewer();
    await show(target, { html: SHORT_DOC, selected: null }, "p0000002");
    const plain = await row(target, "p0000002");
    await show(target, { html: SHORT_DOC, selected: "p0000002" }, "p0000002");
    await settle(() => row(target, "p0000002"), plain, false, "the selected block is marked");
    await show(target, { html: SHORT_DOC, selected: null }, "p0000002");
    await settle(() => row(target, "p0000002"), plain, true, "a block no longer selected is not");
  });

  test("pins show beside the blocks that have comments, with their count", async () => {
    const target = await openViewer();
    await show(target, { html: SHORT_DOC, pins: [] }, "p0000005");
    const plain = await row(target, "p0000001");
    const far = await boxOf(target, "p0000005");
    const textBefore = await target.frame.evaluate(() => document.body.innerText);
    expect(textBefore).not.toMatch(/\d/);

    await show(target, { html: SHORT_DOC, pins: [{ node: "p0000001", count: 2 }] }, "p0000005");
    await settle(() => row(target, "p0000001"), plain, false, "the commented block has a pin");
    await settle(() => boxOf(target, "p0000005"), far, true, "a block without comments has none");
    expect(await target.frame.evaluate(() => document.body.innerText)).toMatch(/(^|\D)2(\D|$)/);
  });

  test("a block named to reveal is scrolled into view", async () => {
    const target = await openViewer();
    await show(target, { html: TALL_DOC }, "p0000160");
    expect(await target.frame.evaluate(() => window.scrollY)).toBe(0);
    await show(target, { html: TALL_DOC, reveal: "p0000150" }, "p0000160");
    await expect
      .poll(
        () =>
          target.frame.evaluate(() => {
            const box = document.querySelector('[data-omni-id="p0000150"]')!.getBoundingClientRect();
            return box.top >= 0 && box.bottom <= window.innerHeight;
          }),
        { timeout: PATIENCE },
      )
      .toBe(true);
  });

  test("a new head of the doc is drawn where the reader had scrolled to", async () => {
    const target = await openViewer();
    await show(target, { html: TALL_DOC }, "p0000160");
    await target.frame.evaluate(() => window.scrollTo(0, 1200));
    expect(await target.frame.evaluate(() => window.scrollY)).toBe(1200);
    const changed = TALL_DOC.replace(
      "</section>",
      '<p data-omni-id="p0000161">A paragraph an agent added at the end.</p>\n</section>',
    );
    await show(target, { html: changed }, "p0000161");
    expect(await target.frame.evaluate(() => window.scrollY)).toBe(1200);
  });
});
