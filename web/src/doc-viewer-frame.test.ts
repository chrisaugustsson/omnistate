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
//   - `html`: the doc's HTML as the API prints it, with `data-node-id` on every
//     block, which the frame shows with the shared stylesheet;
//   - `pins`: `[{ node, count }]`, the blocks that have comments and how many,
//     which the frame marks with a pin beside the block that shows the count;
//   - `selected`: the selected block's node ID, or `null`; the frame shows it
//     as selected;
//   - `claims`: the doc's live claims, as the API lists them (the page shows
//     them too).
//
// What the frame does, tested here:
//
// - Isolation: inside the frame, `parent.document` throws, and no request that
//   the frame makes carries the session. The policy lets nothing but images of
//   the workspace's attachments reach the network: a fetch, an XHR, a beacon,
//   a stylesheet, a font or an image from anywhere else never reaches the
//   server.
// - Only the viewer's own script runs: a script, an event handler attribute or
//   an SVG `onload` inside the doc's HTML never runs.
// - Mermaid figures (`<figure><pre class="mermaid">`) are drawn by the viewer as
//   an SVG in the figure, in place of the Mermaid text.
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
const FULL_DOC = `<section data-node-id="a0000001">
<h1 data-node-id="a0000002">Order service</h1>
<p data-node-id="a0000003">Plain, <strong>strong</strong>, <em>emphasis</em>, <code>code()</code> and <a href="https://example.com/docs">a link</a>.<br>After the line break.</p>
<blockquote data-node-id="a0000004">Measure twice, cut once.</blockquote>
<hr data-node-id="a0000005">
<ul data-node-id="a0000006">
<li data-node-id="a0000007">First item</li>
<li data-node-id="a0000008">Second item</li>
</ul>
<ol data-node-id="a0000009">
<li data-node-id="a0000010">Step one</li>
</ol>
<table data-node-id="a0000011">
<tr data-node-id="a0000012"><th data-node-id="a0000013">Field</th><th data-node-id="a0000014">Type</th></tr>
<tr data-node-id="a0000015"><td data-node-id="a0000016">id</td><td data-node-id="a0000017">uuid</td></tr>
</table>
<figure data-node-id="a0000018"><svg viewBox="0 0 100 50"><rect x="1" y="1" width="50" height="20" fill="#336699"/><circle cx="70" cy="25" r="10"/></svg><figcaption>A box and a circle</figcaption></figure>
<figure data-node-id="a0000019"><pre class="mermaid">graph TD;
  Alpha--&gt;Beta</pre><figcaption>The flow</figcaption></figure>
<aside data-node-id="a0000020" data-callout="note">A note.</aside>
<aside data-node-id="a0000021" data-callout="warning">Mind the gap.</aside>
<aside data-node-id="a0000022" data-callout="decision">Orders are never deleted.</aside>
<pre data-node-id="a0000023"><code>fn main() {
    println!("hi");
}</code></pre>
<img data-node-id="a0000024" src="attachment:${ATTACHMENT_ID}" alt="A screenshot">
<p data-node-id="a0000025" data-block="requirement">FR-1: The system must keep every version.</p>
<article data-node-id="a0000026" data-block="user-story">
<h3 data-node-id="a0000027">Reset a password</h3>
<p data-node-id="a0000028">As a user, I want to reset my password so that I can get back in.</p>
<ul data-node-id="a0000029"><li data-node-id="a0000030">The link works once</li></ul>
</article>
</section>`;

/** A short doc whose blocks all fit in the frame; no digit appears in its text. */
const SHORT_DOC = `<section data-node-id="s0000001">
<h1 data-node-id="h0000001">Checkout</h1>
<p data-node-id="p0000001">Orders ship in <strong>one</strong> day.</p>
<p data-node-id="p0000002">Refunds take a week.</p>
<ul data-node-id="t0000001"><li data-node-id="k0000001">Cards</li><li data-node-id="k0000002">Invoices</li></ul>
<p data-node-id="p0000003">Returns are free.</p>
<p data-node-id="p0000004">Gift wrap is extra.</p>
<p data-node-id="p0000005">Taxes are shown at the end.</p>
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

/** Posts `viewer.show` to the frame and waits until it shows the block `waitFor`. */
async function show(
  target: { page: Page; frame: Frame },
  message: { html: string; pins?: { node: string; count: number }[]; selected?: string | null },
  waitFor: string,
) {
  await target.page.evaluate((data) => {
    const frame = document.getElementById("viewer") as HTMLIFrameElement;
    frame.contentWindow!.postMessage(data, "*");
  }, { type: "viewer.show", pins: [], selected: null, claims: [], ...message });
  await target.frame.waitForSelector(`[data-node-id="${waitFor}"]`, { timeout: PATIENCE });
}

/** The messages the frame posted to the page. */
async function got(target: { page: Page }): Promise<{ type?: string; node?: string }[]> {
  return target.page.evaluate(() => (window as unknown as { got: { type?: string; node?: string }[] }).got);
}

/** A screenshot of the frame's row of the block: the frame's width across the block's height, with a margin. */
async function row(target: { page: Page; frame: Frame }, node: string) {
  const box = await target.frame.locator(`[data-node-id="${node}"]`).boundingBox();
  expect(box, `the block ${node} is shown`).toBeTruthy();
  const top = Math.max(FRAME_TOP, Math.floor(box!.y) - 4);
  const bottom = Math.min(FRAME_TOP + FRAME_HEIGHT, Math.ceil(box!.y + box!.height) + 4);
  return target.page.screenshot({
    clip: { x: FRAME_LEFT, y: top, width: FRAME_WIDTH, height: bottom - top },
  });
}

/** A screenshot of the block's own box. */
async function boxOf(target: { page: Page; frame: Frame }, node: string) {
  return target.frame.locator(`[data-node-id="${node}"]`).screenshot();
}

/** Moves the mouse, in steps as a hand does, to the middle of the block's first line. */
async function pointAt(target: { page: Page; frame: Frame }, node: string) {
  const box = await target.frame.locator(`[data-node-id="${node}"]`).boundingBox();
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
  test("inside the frame, parent.document throws", async () => {
    const target = await openViewer();
    const outcome = await target.frame.evaluate(() => {
      try {
        return `read: ${parent.document.title}`;
      } catch (error) {
        return `threw: ${(error as Error).name}`;
      }
    });
    expect(outcome).toMatch(/^threw/);
  });

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
    for (const request of fromFrame) {
      expect(request.cookie, `${request.path} carries no session`).not.toContain(SESSION_COOKIE);
      expect(
        request.path.startsWith(`/api/v1/w/${WORKSPACE_ID}/attachments/`),
        `${request.path} was fetched, but only the workspace's attachments may be`,
      ).toBe(true);
    }
  });

  test("only the viewer's own script runs: scripts and handlers in the doc do not", async () => {
    const target = await openViewer();
    const html = `<section data-node-id="s0000001">
<p data-node-id="p0000001">Clean text.</p>
<script>parent.postMessage({ type: "injected", via: "script" }, "*")</script>
<img src="data:," onerror='parent.postMessage({ type: "injected", via: "onerror" }, "*")'>
<svg onload='parent.postMessage({ type: "injected", via: "svg" }, "*")'><rect width="5" height="5"/></svg>
<p data-node-id="p0000002" onmouseover='parent.postMessage({ type: "injected", via: "handler" }, "*")'>Hover here.</p>
</section>`;
    await show(target, { html }, "p0000002");
    await pointAt(target, "p0000002");
    await new Promise((resolve) => setTimeout(resolve, 1_000));
    expect((await got(target)).filter((m) => m?.type === "injected")).toEqual([]);
  });

  test("the shared stylesheet styles the doc: the callout kinds look different", async () => {
    const target = await openViewer();
    await show(target, { html: FULL_DOC }, "a0000022");
    const looks = await target.frame.evaluate(() =>
      ["a0000020", "a0000021", "a0000022"].map((id) => {
        const style = getComputedStyle(document.querySelector(`[data-node-id="${id}"]`)!);
        return [style.backgroundColor, style.borderLeftColor, style.borderTopColor, style.color].join(" ");
      }),
    );
    expect(new Set(looks).size, `note, warning and decision callouts: ${JSON.stringify(looks)}`).toBe(3);
  });

  test("Mermaid figures are drawn by the viewer", async () => {
    const target = await openViewer();
    await show(target, { html: FULL_DOC }, "a0000019");
    await target.frame.waitForFunction(
      () => {
        const figure = document.querySelector('[data-node-id="a0000019"]');
        const svg = figure?.querySelector("svg");
        return Boolean(svg && /Alpha/.test(svg.textContent ?? "") && /Beta/.test(svg.textContent ?? ""));
      },
      undefined,
      { timeout: PATIENCE },
    );
    const figure = await target.frame.evaluate(() => {
      const element = document.querySelector('[data-node-id="a0000019"]') as HTMLElement;
      return { text: element.innerText, caption: element.querySelector("figcaption")?.textContent ?? "" };
    });
    expect(figure.text, "the Mermaid source is drawn, not shown as text").not.toMatch(/graph TD/);
    expect(figure.caption).toBe("The flow");
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

    await target.frame.click('[data-node-id="p0000002"]', { force: true });
    await expect.poll(lastSelect, { timeout: PATIENCE }).toBe("p0000002");

    await target.frame.click('[data-node-id="p0000001"] strong', { force: true });
    await expect.poll(lastSelect, { timeout: PATIENCE }).toBe("p0000001");

    await target.frame.click('[data-node-id="k0000002"]', { force: true });
    await expect.poll(lastSelect, { timeout: PATIENCE }).toBe("k0000002");

    await target.frame.click('[data-node-id="h0000001"]', { force: true });
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

  test("the doc shown follows what the page posts, as a live draft does", async () => {
    const target = await openViewer();
    await show(target, { html: SHORT_DOC }, "p0000005");
    const changed = SHORT_DOC.replace(
      '<p data-node-id="p0000005">Taxes are shown at the end.</p>',
      '<p data-node-id="p0000005">Taxes are shown at the end.</p>\n<p data-node-id="p0000006">Coupons stack.</p>',
    );
    await show(target, { html: changed }, "p0000006");
    expect(await target.frame.evaluate(() => document.body.innerText)).toContain("Coupons stack.");
  });
});
