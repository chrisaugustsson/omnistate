# Ordrar benchmark

## Baseline

- Date: 2026-10-30
- Machine: Apple M4 Pro, 24 GB memory, Darwin 25.6.0 (arm64), Chrome 156.0.8078.79
- WebGL renderer: `ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version)`
- Builds: release build of the API (docker/api.Dockerfile), production build of the web app (docker/web.Dockerfile)
- File: fixtures/penpot/tirebud-ordrar.penpot, 797 boards on 5 pages; the pan and zoom is on the page Ordrar (769 boards); imported in 16.7 s
- Chrome: headed, driven by Playwright, 1440x900 viewport, device pixel ratio 2
- Method: 5 runs, each from a new browser with an empty cache and a restarted API container; the numbers are the median of the 5 runs (memory: the highest peak). A frame is complete when the GPU has finished it (`gl.finish()`), and a frame's time is the time from the previous complete frame.
- Path: from the opening board out to zoom 0.010, the viewer's minimum on this page, then across the whole page and back in. At that zoom at most 358 of the 769 boards of the page are in view; the page is too tall for all of them to be, and the file's other 28 boards are on other pages.

| Measurement | Result | FR-28 limit | |
| --- | --- | --- | --- |
| Cold time to first board | 683.1 ms | under 2000 ms | within the limit |
| Frame time over the 10 s pan and zoom, median | 10.8 ms | | |
| Frame time over the 10 s pan and zoom, 95th percentile | 697.7 ms | 16.7 ms or less (60 fps) | over the limit |
| API peak memory | 60.1 MiB | under 512 MiB | within the limit |

### The runs

| Run | First board (ms) | Frames | Frame median (ms) | Frame p95 (ms) | Slowest frame (ms) | Animation frame interval p95 (ms) | Most boards in view | API peak (MiB) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1530.0 | 87 | 10.5 | 722.1 | 1046.4 | 722.2 | 357 | 56.5 |
| 2 | 683.1 | 83 | 12.1 | 710.8 | 1177.9 | 612.3 | 351 | 60.1 |
| 3 | 657.4 | 87 | 10.8 | 691.2 | 1105.2 | 641.0 | 358 | 58.6 |
| 4 | 664.3 | 86 | 10.5 | 697.7 | 1113.1 | 676.7 | 351 | 58.5 |
| 5 | 686.8 | 86 | 11.0 | 635.3 | 1068.1 | 635.2 | 337 | 56.9 |

## Profile

### Cold load, navigation to the first board

Run 2 of 5, the median one: 683.1 ms from the navigation to `/docs/:doc` to the first complete frame that shows the design's first board in full: all of it in view at full detail, read, with its text and images (2 boards in view, 98 nodes painted); a click on it then selects a node, or the run fails. Every instant counts once: for the phase the main thread was in, then for other if it was in another renderer call or a long task, else for the network if a request was open, else for other.

| Phase | ms | Share of the load |
| --- | --- | --- |
| Network | 417.7 | 61.1% |
| Parsing | 82.7 | 12.1% |
| Layout | 34.6 | 5.1% |
| Painting | 16.2 | 2.4% |
| Uploading to the GPU | 59.6 | 8.7% |
| Other (JavaScript, React, other renderer calls, start-up, idle) | 72.3 | 10.6% |

The requests of that load that took longest:

| Request | Start (ms) | Duration (ms) | Transferred (KB) |
| --- | --- | --- | --- |
| `/api/v1/w/:id/projects/:id/docs/:id/html` | 193.6 | 188.9 | 109.3 |
| `/render/omni_render.wasm` | 215.2 | 63.5 | 9445.6 |
| `/api/v1/w/:id/attachments/:id` | 395.3 | 60.4 | 63.0 |
| `/api/v1/w/:id/attachments/:id` | 395.2 | 59.6 | 60.5 |
| `/api/v1/w/:id/attachments/:id` | 395.1 | 58.1 | 64.9 |
| `/api/v1/w/:id/attachments/:id` | 395.0 | 55.8 | 63.1 |

### Pan and zoom frames

Run 4 of 5, the one with the median 95th-percentile frame time: 86 frames over 11119.8 ms. A frame is charged with everything since the previous frame, and its phases add up to its time: handling the wheel events (painting), the JSON that the page reads and writes (parsing), the renderer reading a page and a design system and laying them out (parsing and layout), the draw (painting) and the GPU finishing it. The rest is other: the page's JavaScript, the renderer calls that are in no phase, and the wait for the next animation frame. The network is not in a frame: boards are fetched between frames and show when they arrive.

| Phase | Typical frame, mean (ms) | Share | Slowest 5% of frames, mean (ms) | Share |
| --- | --- | --- | --- | --- |
| Network | not applicable | not applicable | not applicable | not applicable |
| Parsing | 8.7 | 6.8% | 68.5 | 7.9% |
| Layout | 52.4 | 40.5% | 512.9 | 58.9% |
| Painting | 5.7 | 4.4% | 21.3 | 2.5% |
| Uploading to the GPU | 0.7 | 0.5% | 0.4 | 0.0% |
| Other (JavaScript, React, other renderer calls, start-up, idle) | 61.8 | 47.8% | 267.7 | 30.7% |
| Frame | 129.3 | 100.0% | 870.8 | 100.0% |

The renderer calls of that run that took longest:

| Call | Calls | Total (ms) | Share of the path |
| --- | --- | --- | --- |
| `_omni_render_scene_load` | 11 | 2831.8 | 25.5% |
| `_omni_render_scene_set_system` | 11 | 2371.1 | 21.3% |
| `_omni_render_scene_layers` | 11 | 562.3 | 5.1% |
| `_omni_render_scene_draw` | 86 | 546.6 | 4.9% |
| `_omni_render_visible_boards` | 150 | 3.1 | 0.0% |
| `_omni_render_overlay` | 85 | 0.6 | 0.0% |

## Reading the profile

Written by hand from the tables above; `make bench-ordrar` keeps this section and rewrites the rest.

- **The clock stops at the first board complete, not at the first thing drawn.** The cold time ends at the first complete frame where the first board of the design's manifest is read, all in view at full detail, so with its text as text, with no image missing; each run then clicks on it and fails unless a node is selected. A design viewer page used to open fitted to all its boards, which on Omslag (3038 by 62521 px) is a speck of about 21 by 10 px at zoom 0.013 with its text as bars. A page whose boards would be drawn at reduced detail now opens fitted to its first board (`Scene::opening`), a link to a node moves the view to that node's board, and pages that can be read whole open as before.
- **The cold load is still mostly the network, but a much shorter one.** The longest request of the load is the design's outline (`GET .../docs/:doc/html?view=outline`), which the viewer needs before it can show anything. It was about 1 MB and 800 ms. The boxes of the placeholders (`crates/omni-api/src/placeholders.rs`) are laid out in the background when a design is written and stored in RustFS, one object per design system version, so that the first outline after a restart reads them instead of laying out about 550 of the 797 boards (the API's part of the request went from about 500 ms to about 120 ms). A box that is not stored yet, or was laid out against an older design system, is laid out when the outline is read, as before. And the proxy compresses API responses (`encode zstd gzip` in `docker/Caddyfile`), which takes the outline from about 1 MB to about 110 KB. The first run after the stack starts is slower (here 1530 ms against 660 to 690 ms), and a median can hide that, so compare the runs too. After the outline, the viewer waits for the images of the page before it shows it, then reads the boards in view. Parsing, layout, painting and the GPU together are about a quarter of the load.
- **Panning and zooming stalls on `renderer.show`, and half of that is wasted.** The viewer shows the page again with every board that has arrived, about eleven times on the path, and each show holds the main thread for about 0.5 s or more. That is what the 95th percentile is: the frame after a show. `scene_set_system` re-lays out the page that is loaded (about 215 ms per show), and `scene_load` straight after it replaces that page and lays it out again (about 255 ms), so the first layout is thrown away. Layout is about four tenths of the time over the path and six tenths of a stalled frame. The shows are cheaper than in the first baseline because the page opens on one board, so fewer boards are read before the path.
- **Almost half of the path is in other, and it is not idle.** A one-off CPU profile of the path, not part of `make bench-ordrar`, shows the main thread idle for only about 150 ms. The largest part of other, 2.1 to 2.5 s per path, is the browser's own page layout: `drawFrame` in `web/src/design/renderer.ts` reads `canvas.clientWidth`, which forces a layout of the whole page, single reads taking up to 800 ms. The reads this long follow the shows; the likely cause is the layers panel of over 20,000 rows that React has just rendered, which the profile cannot confirm. In the renderer, `scene_layers` adds about 0.7 s per path (see the table of calls), and the rest of other is the browser's other work and the page's JavaScript.
- **Drawing is not the bottleneck.** Painting is about 4% of the path and uploading to the GPU under 1%. Between stalls the viewer keeps up: the median frame is about 10 ms, which includes the wait for the next animation frame. The frames are few (about 85 in 10 s) because the main thread is held, and the path overruns its 10 s because the last step waits for the last stall.
- **Memory is within the limit with room to spare.** The API's peak is the highest sample of the container's memory as `docker stats` reports it, about once a second, from before the navigation to the end of the path, after a restart, so the import is not in it. The figure is the highest peak of the five runs, not their median. A spike shorter than a second can fall between two samples. It was about 250 MiB when the outline laid out the placeholders' boxes itself. The import itself leaves the API near 2 GB resident, which the restart discards.
- **The path.** The file's 797 boards are spread over five pages and the viewer shows one page at a time, so the pan and zoom runs on the page with 769 boards; the file opens on the first page, of 5 boards, whose first board is the one of the cold measurement. That page cannot have all its boards in view: it is 8118 by 329640 px, which would need a zoom of about 0.0024, and the viewer stops at 0.01 (`MIN_ZOOM`). So the path zooms out to 0.01 and pans the whole page from there, with about 350 of the 769 boards in view at once. The "most boards in view" of a run depends on where its few frames land, so it differs between runs.
- **Limits of the method.** A frame's time is the time from the previous complete frame in the running viewer, so unlike renderer gate A, which draws frames back to back, it includes the wait for the next animation frame. Layout is the renderer's and includes resolving instances and tokens, because it does both in one step; the browser's page layout is in other. Handling the wheel events counts as painting. Uploading to the GPU is Skia submitting its commands plus the wait for `gl.finish()`; the GPU's own execution time is not separated from it. In the cold table, script that runs in a task shorter than 50 ms while a request is open counts as network, because only the renderer's calls and long tasks are seen. The renderer's phase times come from `_omni_render_scene_timings`, added for this bench.
