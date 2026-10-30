# Ordrar benchmark

## Baseline

- Date: 2026-10-30
- Machine: Apple M4 Pro, 24 GB memory, Darwin 25.6.0 (arm64), Chrome 156.0.8078.79
- WebGL renderer: `ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version)`
- Builds: release build of the API (docker/api.Dockerfile), production build of the web app (docker/web.Dockerfile)
- File: fixtures/penpot/tirebud-ordrar.penpot, 797 boards on 5 pages; the pan and zoom is on the page Ordrar (769 boards); imported in 13.5 s
- Chrome: headed, driven by Playwright, 1440x900 viewport, device pixel ratio 2
- Method: 5 runs, each from a new browser with an empty cache and a restarted API container; the numbers are the median of the 5 runs (memory: the highest peak). A frame is complete when the GPU has finished it (`gl.finish()`).

| Measurement | Result | FR-28 limit | |
| --- | --- | --- | --- |
| Cold time to first board | 1055.5 ms | under 2000 ms | within the limit |
| Frame time over the 10 s pan and zoom, median | 1.9 ms | | |
| Frame time over the 10 s pan and zoom, 95th percentile | 639.3 ms | 16.7 ms or less (60 fps) | over the limit |
| API peak memory | 278.3 MiB | under 512 MiB | within the limit |

### The runs

| Run | First board (ms) | Frames | Frame median (ms) | Frame p95 (ms) | Slowest frame (ms) | Animation frame interval p95 (ms) | API peak (MiB) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1240.0 | 33 | 2.0 | 654.5 | 666.5 | 1365.8 | 243.8 |
| 2 | 1055.5 | 29 | 2.2 | 637.4 | 646.6 | 2102.8 | 246.6 |
| 3 | 1050.1 | 37 | 1.7 | 639.3 | 658.3 | 1379.7 | 248.6 |
| 4 | 1031.9 | 32 | 1.9 | 655.2 | 667.6 | 1432.1 | 278.3 |
| 5 | 1084.2 | 36 | 1.7 | 634.8 | 639.3 | 1368.8 | 245.4 |

## Profile

### Cold load, navigation to the first board

Run 2 of 5, the median one: 1055.5 ms from the navigation to `/docs/:doc` to the first complete frame that draws a board that is read (5 boards in view, 107 nodes painted). Every instant counts once: for the phase the main thread was in, else for the network if a request was open, else for other.

| Phase | ms | Share of the load |
| --- | --- | --- |
| Network | 838.3 | 79.4% |
| Parsing | 34.1 | 3.2% |
| Layout | 29.7 | 2.8% |
| Painting | 24.0 | 2.3% |
| Uploading to the GPU | 13.5 | 1.3% |
| Other (JavaScript, React, WebAssembly start-up, idle) | 115.9 | 11.0% |

The requests of that load that took longest:

| Request | Start (ms) | Duration (ms) | Transferred (KB) |
| --- | --- | --- | --- |
| `/api/v1/w/:id/projects/:id/docs/:id/html` | 166.5 | 637.0 | 1013.5 |
| `/render/omni_render.wasm` | 186.8 | 56.8 | 9442.6 |
| `/assets/w._workspaceId-D7Xq5RCn.js` | 19.9 | 50.9 | 0.7 |
| `/api/v1/w/:id/attachments/:id` | 816.6 | 50.7 | 127.1 |
| `/api/v1/w/:id/attachments/:id` | 816.5 | 50.6 | 124.7 |
| `/assets/search-D6Znnd9w.js` | 19.9 | 50.4 | 2.1 |

### Pan and zoom frames

Run 3 of 5, the one with the median 95th-percentile frame time: 37 frames. A frame is charged with everything that ran since the previous frame: handling the wheel events, the JSON that the page reads and writes, loading a page into the renderer (parsing and layout), the draw (painting) and the GPU finishing it. The network is not in a frame: boards are fetched between frames and show when they arrive.

| Phase | Typical frame, mean (ms) | Share | Slowest 5% of frames, mean (ms) | Share |
| --- | --- | --- | --- | --- |
| Network | not applicable | not applicable | not applicable | not applicable |
| Parsing | 11.8 | 13.5% | 88.7 | 13.7% |
| Layout | 65.8 | 75.3% | 508.8 | 78.4% |
| Painting | 9.0 | 10.3% | 50.8 | 7.8% |
| Uploading to the GPU | 0.8 | 0.9% | 0.6 | 0.1% |
| Frame | 87.3 | 100.0% | 648.8 | 100.0% |

## Reading the profile

Written by hand from the tables above; `make bench-ordrar` keeps this section and rewrites the rest.

- **The cold load is dominated by one request.** About four fifths of the time to first board is the network phase, and the longest request in it is the design's outline (`GET .../docs/:doc/html?view=outline`, about 1 MB, over 600 ms), which the viewer needs before it can show anything. It starts after the page's own scripts and the 9 MB renderer have begun loading, and the first board is drawn some 150 ms after it ends. Parsing, layout, painting and the GPU together are about a tenth of the load; the rest is JavaScript, React and WebAssembly start-up. The first run after a restart is often the slowest of the five.
- **Panning and zooming is dominated by layout, not by drawing.** The viewer shows the page again with every board that has arrived (`renderer.show`), and each show lays the whole page out from scratch, 769 boards with all that are read, so a frame that follows one costs hundreds of milliseconds. Layout is three quarters of a typical frame and over three quarters of the slowest ones; painting is about a tenth and uploading to the GPU under 1%. Drawing alone is cheap: the median frame is a couple of milliseconds, which is why the median and the mean of the frames differ so much. The 95th percentile (several hundred ms) is the figure that misses FR-28's 60 fps, and the frames are few (about 30 in 10 s) because the main thread is held by those loads.
- **Memory is within the limit with room to spare.** The API's peak is the process's resident memory (`VmHWM`) from before the navigation to the end of the path, after a restart, so the import is not in it. It is about half of FR-28's limit. The import itself leaves the API near 2 GB resident, which the restart discards.
- **The path.** The file's 797 boards are spread over five pages and the viewer shows one page at a time, so the pan and zoom runs on the page with 769 boards; the file opens on the first page, a 5-board cover, which is the first board of the cold measurement.
- **Limits of the method.** Layout includes resolving instances and tokens, because the renderer does both in one step when a page is loaded. Uploading to the GPU is Skia submitting its commands plus the wait for `gl.finish()`; the GPU's own execution time is not separated from it. The renderer's phase times come from `_omni_render_scene_timings`, added for this bench.
