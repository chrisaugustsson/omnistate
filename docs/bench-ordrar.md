# Ordrar benchmark

## Baseline

- Date: 2026-10-30
- Machine: Apple M4 Pro, 24 GB memory, Darwin 25.6.0 (arm64), Chrome 156.0.8078.79
- WebGL renderer: `ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version)`
- Builds: release build of the API (docker/api.Dockerfile), production build of the web app (docker/web.Dockerfile)
- File: fixtures/penpot/tirebud-ordrar.penpot, 797 boards on 5 pages; the pan and zoom is on the page Ordrar (769 boards); imported in 15.3 s
- Chrome: headed, driven by Playwright, 1440x900 viewport, device pixel ratio 2
- Method: 5 runs, each from a new browser with an empty cache and a restarted API container; the numbers are the median of the 5 runs (memory: the highest peak). A frame is complete when the GPU has finished it (`gl.finish()`), and a frame's time is the time from the previous complete frame.
- Path: from the opening board out to zoom 0.010, the viewer's minimum on this page, then across the whole page and back in. At that zoom at most 356 of the 769 boards of the page are in view; the page is too tall for all of them to be, and the file's other 28 boards are on other pages.

| Measurement | Result | FR-28 limit | |
| --- | --- | --- | --- |
| Cold time to first board | 665.3 ms | under 2000 ms | within the limit |
| Frame time over the 10 s pan and zoom, median | 10.8 ms | | |
| Frame time over the 10 s pan and zoom, 95th percentile | 1350.3 ms | 16.7 ms or less (60 fps) | over the limit |
| API peak memory | 70.3 MiB | under 512 MiB | within the limit |

### The runs

| Run | First board (ms) | Frames | Frame median (ms) | Frame p95 (ms) | Slowest frame (ms) | Animation frame interval p95 (ms) | Most boards in view | API peak (MiB) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 788.2 | 39 | 10.2 | 1489.3 | 1490.3 | 1477.1 | 348 | 70.3 |
| 2 | 696.0 | 38 | 11.0 | 1435.7 | 1490.9 | 1440.1 | 356 | 67.7 |
| 3 | 629.1 | 38 | 11.6 | 1350.3 | 1487.0 | 1346.2 | 320 | 68.1 |
| 4 | 665.3 | 41 | 10.8 | 1285.2 | 1476.6 | 1281.9 | 350 | 68.0 |
| 5 | 635.0 | 45 | 10.1 | 1274.6 | 1441.0 | 1274.6 | 316 | 67.5 |

## Profile

### Cold load, navigation to the first board

Run 4 of 5, the median one: 665.3 ms from the navigation to `/docs/:doc` to the first complete frame that draws a board that is read (5 boards in view, 107 nodes painted). Every instant counts once: for the phase the main thread was in, then for other if it was in another renderer call or a long task, else for the network if a request was open, else for other.

| Phase | ms | Share of the load |
| --- | --- | --- |
| Network | 439.5 | 66.1% |
| Parsing | 84.1 | 12.6% |
| Layout | 35.6 | 5.4% |
| Painting | 26.5 | 4.0% |
| Uploading to the GPU | 14.2 | 2.1% |
| Other (JavaScript, React, other renderer calls, start-up, idle) | 65.4 | 9.8% |

The requests of that load that took longest:

| Request | Start (ms) | Duration (ms) | Transferred (KB) |
| --- | --- | --- | --- |
| `/api/v1/w/:id/projects/:id/docs/:id/html` | 212.4 | 183.0 | 109.2 |
| `/assets/search-D6Znnd9w.js` | 31.1 | 76.3 | 2.1 |
| `/assets/w._workspaceId-D7Xq5RCn.js` | 31.1 | 76.3 | 0.7 |
| `/assets/file-text-DZShRado.js` | 31.1 | 75.9 | 0.7 |
| `/assets/layers-D8hNnfCI.js` | 31.1 | 75.8 | 0.8 |
| `/assets/visits-BlbCjeQC.js` | 31.0 | 73.8 | 1.2 |

### Pan and zoom frames

Run 3 of 5, the one with the median 95th-percentile frame time: 38 frames over 11773.7 ms. A frame is charged with everything since the previous frame, and its phases add up to its time: handling the wheel events (painting), the JSON that the page reads and writes (parsing), the renderer reading a page and a design system and laying them out (parsing and layout), the draw (painting) and the GPU finishing it. The rest is other: the page's JavaScript, the renderer calls that are in no phase, and the wait for the next animation frame. The network is not in a frame: boards are fetched between frames and show when they arrive.

| Phase | Typical frame, mean (ms) | Share | Slowest 5% of frames, mean (ms) | Share |
| --- | --- | --- | --- | --- |
| Network | not applicable | not applicable | not applicable | not applicable |
| Parsing | 15.2 | 4.9% | 119.2 | 8.4% |
| Layout | 131.4 | 42.4% | 1066.5 | 75.2% |
| Painting | 11.6 | 3.7% | 48.0 | 3.4% |
| Uploading to the GPU | 0.7 | 0.2% | 0.8 | 0.1% |
| Other (JavaScript, React, other renderer calls, start-up, idle) | 150.9 | 48.7% | 184.1 | 13.0% |
| Frame | 309.8 | 100.0% | 1418.6 | 100.0% |

The renderer calls of that run that took longest:

| Call | Calls | Total (ms) | Share of the path |
| --- | --- | --- | --- |
| `_omni_render_scene_load` | 5 | 2890.6 | 24.6% |
| `_omni_render_scene_set_system` | 5 | 2631.4 | 22.3% |
| `_omni_render_scene_layers` | 5 | 711.1 | 6.0% |
| `_omni_render_scene_draw` | 38 | 467.6 | 4.0% |
| `_omni_render_visible_boards` | 62 | 1.6 | 0.0% |
| `_omni_render_overlay` | 37 | 0.5 | 0.0% |

## Reading the profile

Written by hand from the tables above; `make bench-ordrar` keeps this section and rewrites the rest.

- **The cold load is still mostly the network, but a much shorter one.** The longest request of the load is the design's outline (`GET .../docs/:doc/html?view=outline`), which the viewer needs before it can show anything. It was about 1 MB and 800 ms, and two changes took the time to first board from about 1000 ms to about 650 ms. The boxes of the placeholders (`crates/omni-api/src/placeholders.rs`) are laid out in the background when a design is written and stored in RustFS, one object per design system version, so that the first outline after a restart reads them instead of laying out about 550 of the 797 boards (the API's part of the request went from about 500 ms to about 120 ms). A box that is not stored yet, or was laid out against an older design system, is laid out when the outline is read, as before. And the proxy compresses API responses (`encode zstd gzip` in `docker/Caddyfile`), which takes the outline from about 1 MB to about 110 KB. The first run after a restart is slower (here 790 ms against 630 to 700 ms), as it was before. After the outline, the viewer waits for the images of the page before it shows it. Parsing, layout, painting and the GPU together are about a quarter of the load.
- **Panning and zooming stalls on `renderer.show`, and half of that is wasted.** The viewer shows the page again with every board that has arrived, about five times on the path, and each show holds the main thread for about 1.3 s. That is what the 95th percentile is: the frame after a show. `scene_set_system` re-lays out the page that is loaded (about 530 ms per show), and `scene_load` straight after it replaces that page and lays it out again (about 560 ms), so the first layout is thrown away. Layout is about four tenths of the time over the path and three quarters of a stalled frame.
- **Almost half of the path is in other, and it is not idle.** A one-off CPU profile of the path, not part of `make bench-ordrar`, shows the main thread idle for only about 150 ms. The largest part of other, 2.1 to 2.5 s per path, is the browser's own page layout: `drawFrame` in `web/src/design/renderer.ts` reads `canvas.clientWidth`, which forces a layout of the whole page, single reads taking up to 800 ms. The reads this long follow the shows; the likely cause is the layers panel of over 20,000 rows that React has just rendered, which the profile cannot confirm. In the renderer, `scene_layers` adds about 0.7 s per path (see the table of calls), and the rest of other is the browser's other work and the page's JavaScript.
- **Drawing is not the bottleneck.** Painting is about 4% of the path and uploading to the GPU under 1%. Between stalls the viewer keeps up: the median frame is about 10 ms, which includes the wait for the next animation frame. The frames are few (about 40 in 10 s) because the main thread is held, and the path overruns its 10 s because the last step waits for the last stall. Now and then a run is much worse, as run 2 of this baseline: 22 frames, with even the median frame at 155 ms.
- **Memory is within the limit with room to spare.** The API's peak is the process's resident memory (`VmHWM`, of PID 1 in the container) from before the navigation to the end of the path, after a restart, so the import is not in it. The figure is the highest peak of the five runs, not their median. It is about a seventh of FR-28's limit; it was about 250 MiB when the outline laid out the placeholders' boxes itself. The import itself leaves the API near 2 GB resident, which the restart discards.
- **The path.** The file's 797 boards are spread over five pages and the viewer shows one page at a time, so the pan and zoom runs on the page with 769 boards; the file opens on the first page, a 5-board cover, which is the first board of the cold measurement. That page cannot have all its boards in view: it is 8118 by 329640 px, which would need a zoom of about 0.0024, and the viewer stops at 0.01 (`MIN_ZOOM`). So the path zooms out to 0.01 and pans the whole page from there, with about 350 of the 769 boards in view at once. The "most boards in view" of a run depends on where its few frames land, so it differs between runs.
- **Limits of the method.** A frame's time is the time from the previous complete frame in the running viewer, so unlike renderer gate A, which draws frames back to back, it includes the wait for the next animation frame. Layout is the renderer's and includes resolving instances and tokens, because it does both in one step; the browser's page layout is in other. Handling the wheel events counts as painting. Uploading to the GPU is Skia submitting its commands plus the wait for `gl.finish()`; the GPU's own execution time is not separated from it. In the cold table, script that runs in a task shorter than 50 ms while a request is open counts as network, because only the renderer's calls and long tasks are seen. The renderer's phase times come from `_omni_render_scene_timings`, added for this bench.
