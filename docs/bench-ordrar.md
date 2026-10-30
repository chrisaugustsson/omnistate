# Ordrar benchmark

## Baseline

- Date: 2026-10-30
- Machine: Apple M4 Pro, 24 GB memory, Darwin 25.6.0 (arm64), Chrome 156.0.8078.79
- WebGL renderer: `ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version)`
- Builds: release build of the API (docker/api.Dockerfile), production build of the web app (docker/web.Dockerfile)
- File: fixtures/penpot/tirebud-ordrar.penpot, 797 boards on 5 pages; the pan and zoom is on the page Ordrar (769 boards); imported in 16.7 s
- Chrome: headed, driven by Playwright, 1440x900 viewport, device pixel ratio 2
- Method: 5 runs, each from a new browser with an empty cache and a restarted API container; the numbers are the median of the 5 runs (memory: the highest peak). A frame is complete when the GPU has finished it (`gl.finish()`), and a frame's time is the time from the previous complete frame.
- Path: from the opening board out to zoom 0.010, the viewer's minimum on this page, then across the whole page and back in. At that zoom at most 356 of the 769 boards of the page are in view; the page is too tall for all of them to be, and the file's other 28 boards are on other pages.

| Measurement | Result | FR-28 limit | |
| --- | --- | --- | --- |
| Cold time to first board | 1271.5 ms | under 2000 ms | within the limit |
| Frame time over the 10 s pan and zoom, median | 12.3 ms | | |
| Frame time over the 10 s pan and zoom, 95th percentile | 1321.6 ms | 16.7 ms or less (60 fps) | over the limit |
| API peak memory | 247.5 MiB | under 512 MiB | within the limit |

### The runs

| Run | First board (ms) | Frames | Frame median (ms) | Frame p95 (ms) | Slowest frame (ms) | Animation frame interval p95 (ms) | Most boards in view | API peak (MiB) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1694.2 | 37 | 12.4 | 1333.5 | 1441.1 | 1329.7 | 356 | 236.6 |
| 2 | 1271.5 | 22 | 155.4 | 1569.5 | 2007.6 | 2008.6 | 347 | 236.6 |
| 3 | 1200.3 | 42 | 10.7 | 1303.6 | 1597.2 | 1299.9 | 320 | 247.5 |
| 4 | 1100.3 | 40 | 12.3 | 1310.7 | 1480.4 | 1365.8 | 343 | 244.5 |
| 5 | 1388.1 | 43 | 10.4 | 1321.6 | 1535.2 | 1317.6 | 290 | 245.5 |

## Profile

### Cold load, navigation to the first board

Run 2 of 5, the median one: 1271.5 ms from the navigation to `/docs/:doc` to the first complete frame that draws a board that is read (5 boards in view, 107 nodes painted). Every instant counts once: for the phase the main thread was in, then for other if it was in another renderer call or a long task, else for the network if a request was open, else for other.

| Phase | ms | Share of the load |
| --- | --- | --- |
| Network | 1041.9 | 81.9% |
| Parsing | 85.6 | 6.7% |
| Layout | 34.9 | 2.7% |
| Painting | 25.3 | 2.0% |
| Uploading to the GPU | 14.3 | 1.1% |
| Other (JavaScript, React, other renderer calls, start-up, idle) | 69.5 | 5.5% |

The requests of that load that took longest:

| Request | Start (ms) | Duration (ms) | Transferred (KB) |
| --- | --- | --- | --- |
| `/api/v1/w/:id/projects/:id/docs/:id/html` | 203.2 | 801.1 | 1013.5 |
| `/render/omni_render.wasm` | 230.3 | 80.0 | 9443.0 |
| `/assets/search-D6Znnd9w.js` | 22.9 | 63.1 | 2.1 |
| `/assets/w._workspaceId-D7Xq5RCn.js` | 23.0 | 63.1 | 0.7 |
| `/assets/file-text-DZShRado.js` | 22.9 | 62.3 | 0.7 |
| `/api/v1/w/:id/attachments/:id` | 1017.6 | 61.6 | 127.1 |

### Pan and zoom frames

Run 5 of 5, the one with the median 95th-percentile frame time: 43 frames over 12010.0 ms. A frame is charged with everything since the previous frame, and its phases add up to its time: handling the wheel events (painting), the JSON that the page reads and writes (parsing), the renderer reading a page and a design system and laying them out (parsing and layout), the draw (painting) and the GPU finishing it. The rest is other: the page's JavaScript, the renderer calls that are in no phase, and the wait for the next animation frame. The network is not in a frame: boards are fetched between frames and show when they arrive.

| Phase | Typical frame, mean (ms) | Share | Slowest 5% of frames, mean (ms) | Share |
| --- | --- | --- | --- | --- |
| Network | not applicable | not applicable | not applicable | not applicable |
| Parsing | 14.0 | 5.0% | 127.6 | 9.1% |
| Layout | 115.3 | 41.3% | 1072.0 | 76.6% |
| Painting | 10.8 | 3.9% | 35.5 | 2.5% |
| Uploading to the GPU | 0.7 | 0.3% | 0.7 | 0.1% |
| Other (JavaScript, React, other renderer calls, start-up, idle) | 138.5 | 49.6% | 164.1 | 11.7% |
| Frame | 279.3 | 100.0% | 1399.9 | 100.0% |

The renderer calls of that run that took longest:

| Call | Calls | Total (ms) | Share of the path |
| --- | --- | --- | --- |
| `_omni_render_scene_load` | 5 | 2868.5 | 23.9% |
| `_omni_render_scene_set_system` | 5 | 2636.9 | 22.0% |
| `_omni_render_scene_layers` | 5 | 688.1 | 5.7% |
| `_omni_render_scene_draw` | 43 | 496.5 | 4.1% |
| `_omni_render_visible_boards` | 67 | 1.4 | 0.0% |
| `_omni_render_alloc` | 10 | 0.9 | 0.0% |

## Reading the profile

Written by hand from the tables above; `make bench-ordrar` keeps this section and rewrites the rest.

- **The cold load is dominated by the network.** About four fifths of the time to first board is the network phase. The longest request in it is the design's outline (`GET .../docs/:doc/html?view=outline`, about 1 MB, 640 to 830 ms), which the viewer needs before it can show anything. It starts after the page's own scripts and the 9 MB renderer have begun loading. The viewer then waits for the images and fonts of the page before it shows it, which is usually tens of milliseconds and in the slow runs over 300 ms. Those slow runs are why the time to first board is not steady: single runs go from about 1.0 s to 1.7 s, and a median of five can land on either side, so one median is not a precise figure. Parsing, layout, painting and the GPU together are about a tenth of the load.
- **Panning and zooming stalls on `renderer.show`, and half of that is wasted.** The viewer shows the page again with every board that has arrived, about five times on the path, and each show holds the main thread for about 1.3 s. That is what the 95th percentile is: the frame after a show. `scene_set_system` re-lays out the page that is loaded (about 530 ms per show), and `scene_load` straight after it replaces that page and lays it out again (about 560 ms), so the first layout is thrown away. Layout is about four tenths of the time over the path and three quarters of a stalled frame.
- **Almost half of the path is in other, and it is not idle.** A one-off CPU profile of the path, not part of `make bench-ordrar`, shows the main thread idle for only about 150 ms. The largest part of other, 2.1 to 2.5 s per path, is the browser's own page layout: `drawFrame` in `web/src/design/renderer.ts` reads `canvas.clientWidth`, which forces a layout of the whole page, single reads taking up to 800 ms. The reads this long follow the shows; the likely cause is the layers panel of over 20,000 rows that React has just rendered, which the profile cannot confirm. In the renderer, `scene_layers` adds about 0.7 s per path (see the table of calls), and the rest of other is the browser's other work and the page's JavaScript.
- **Drawing is not the bottleneck.** Painting is about 4% of the path and uploading to the GPU under 1%. Between stalls the viewer keeps up: the median frame is about 10 ms, which includes the wait for the next animation frame. The frames are few (about 40 in 10 s) because the main thread is held, and the path overruns its 10 s because the last step waits for the last stall. Now and then a run is much worse, as run 2 of this baseline: 22 frames, with even the median frame at 155 ms.
- **Memory is within the limit with room to spare.** The API's peak is the process's resident memory (`VmHWM`, of PID 1 in the container) from before the navigation to the end of the path, after a restart, so the import is not in it. The figure is the highest peak of the five runs, not their median. It is about half of FR-28's limit. The import itself leaves the API near 2 GB resident, which the restart discards.
- **The path.** The file's 797 boards are spread over five pages and the viewer shows one page at a time, so the pan and zoom runs on the page with 769 boards; the file opens on the first page, a 5-board cover, which is the first board of the cold measurement. That page cannot have all its boards in view: it is 8118 by 329640 px, which would need a zoom of about 0.0024, and the viewer stops at 0.01 (`MIN_ZOOM`). So the path zooms out to 0.01 and pans the whole page from there, with about 350 of the 769 boards in view at once. The "most boards in view" of a run depends on where its few frames land, so it differs between runs.
- **Limits of the method.** A frame's time is the time from the previous complete frame in the running viewer, so unlike renderer gate A, which draws frames back to back, it includes the wait for the next animation frame. Layout is the renderer's and includes resolving instances and tokens, because it does both in one step; the browser's page layout is in other. Handling the wheel events counts as painting. Uploading to the GPU is Skia submitting its commands plus the wait for `gl.finish()`; the GPU's own execution time is not separated from it. In the cold table, script that runs in a task shorter than 50 ms while a request is open counts as network, because only the renderer's calls and long tasks are seen. The renderer's phase times come from `_omni_render_scene_timings`, added for this bench.
