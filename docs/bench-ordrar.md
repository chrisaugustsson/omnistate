# Ordrar benchmark

## Baseline

- Date: 2026-10-31
- Machine: Apple M4 Pro, 24 GB memory, Darwin 25.6.0 (arm64), Chrome 156.0.8078.79
- WebGL renderer: `ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version)`
- Builds: release build of the API (docker/api.Dockerfile), production build of the web app (docker/web.Dockerfile)
- File: fixtures/penpot/tirebud-ordrar.penpot, 797 boards on 5 pages; the pan and zoom is on the page Ordrar (769 boards); imported in 16.6 s
- Chrome: headed, driven by Playwright, 1440x900 viewport, device pixel ratio 2
- Method: 5 runs, each from a new browser with an empty cache and a restarted API container; the numbers are the median of the 5 runs (memory: the highest peak). A frame is complete when the GPU has finished it (`gl.finish()`), and a frame's time is the time from the previous complete frame.
- Path: from the opening board out to zoom 0.010, the viewer's minimum on this page, then across the whole page and back in. At that zoom at most 357 of the 769 boards of the page are in view; the page is too tall for all of them to be, and the file's other 28 boards are on other pages.

| Measurement | Result | FR-28 limit | |
| --- | --- | --- | --- |
| Cold time to first board | 745.3 ms | under 2000 ms | within the limit |
| Frame time over the 10 s pan and zoom, median | 10.4 ms | | |
| Frame time over the 10 s pan and zoom, 95th percentile | 13.8 ms | 16.7 ms or less (60 fps) | within the limit |
| API peak memory | 68.8 MiB | under 512 MiB | within the limit |

### The runs

| Run | First board (ms) | Frames | Frame median (ms) | Frame p95 (ms) | Slowest frame (ms) | Animation frame interval p95 (ms) | Most boards in view | API peak (MiB) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 875.3 | 1001 | 10.4 | 14.0 | 28.2 | 14.0 | 357 | 68.8 |
| 2 | 745.3 | 1001 | 10.3 | 13.7 | 28.0 | 13.6 | 357 | 63.8 |
| 3 | 656.6 | 1000 | 10.4 | 13.8 | 32.0 | 13.8 | 356 | 65.5 |
| 4 | 805.3 | 1000 | 10.6 | 14.1 | 25.2 | 14.1 | 357 | 64.1 |
| 5 | 696.7 | 1000 | 10.3 | 13.8 | 24.1 | 13.9 | 357 | 63.3 |

## Profile

### Cold load, navigation to the first board

Run 2 of 5, the median one: 745.3 ms from the navigation to `/docs/:doc` to the first complete frame that shows the design's first board in full: all of it in view at full detail, read, with its text and images (2 boards in view, 66 nodes painted); a click on it then selects a node, or the run fails. Every instant counts once: for the phase the main thread was in, then for other if it was in another renderer call or a long task, else for the network if a request was open, else for other.

| Phase | ms | Share of the load |
| --- | --- | --- |
| Network | 481.3 | 64.6% |
| Parsing | 50.1 | 6.7% |
| Layout | 8.6 | 1.2% |
| Painting | 18.9 | 2.5% |
| Uploading to the GPU | 62.0 | 8.3% |
| Other (JavaScript, React, other renderer calls, start-up, idle) | 124.4 | 16.7% |

The requests of that load that took longest:

| Request | Start (ms) | Duration (ms) | Transferred (KB) |
| --- | --- | --- | --- |
| `/api/v1/w/:id/projects/:id/docs/:id/html` | 216.1 | 213.8 | 109.4 |
| `/render/omni_render.wasm` | 242.3 | 77.3 | 9570.3 |
| `document` | 0.0 | 69.6 | 4.3 |
| `/assets/file-text-BYBdVatO.js` | 72.1 | 60.2 | 0.7 |
| `/assets/search-dhewPUQh.js` | 72.2 | 60.0 | 2.1 |
| `/api/v1/w/:id/projects/:id/docs/:id/html` | 443.7 | 59.7 | 2.3 |

### Pan and zoom frames

Run 5 of 5, the one with the median 95th-percentile frame time: 1000 frames over 10012.9 ms. A frame is charged with everything since the previous frame, and its phases add up to its time: handling the wheel events (painting), the JSON that the page reads and writes (parsing), the renderer reading a page and a design system and laying them out (parsing and layout), the draw (painting) and the GPU finishing it. The rest is other: the page's JavaScript, the renderer calls that are in no phase, and the wait for the next animation frame. The network is not in a frame: boards are fetched between frames and show when they arrive.

| Phase | Typical frame, mean (ms) | Share | Slowest 5% of frames, mean (ms) | Share |
| --- | --- | --- | --- | --- |
| Network | not applicable | not applicable | not applicable | not applicable |
| Parsing | 0.0 | 0.4% | 0.1 | 0.3% |
| Layout | 0.0 | 0.0% | 0.0 | 0.0% |
| Painting | 0.6 | 6.2% | 2.1 | 13.6% |
| Uploading to the GPU | 0.4 | 3.6% | 1.0 | 6.4% |
| Other (JavaScript, React, other renderer calls, start-up, idle) | 9.0 | 89.7% | 12.3 | 79.6% |
| Frame | 10.0 | 100.0% | 15.4 | 100.0% |

The renderer calls of that run that took longest:

| Call | Calls | Total (ms) | Share of the path |
| --- | --- | --- | --- |
| `_omni_render_scene_update_step` | 1754 | 1474.7 | 14.7% |
| `_omni_render_scene_draw` | 1000 | 986.8 | 9.9% |
| `_omni_render_scene_fonts` | 775 | 333.0 | 3.3% |
| `_omni_render_scene_update_boards` | 533 | 140.0 | 1.4% |
| `_omni_render_visible_boards` | 1744 | 39.9 | 0.4% |
| `_omni_render_overlay` | 999 | 4.4 | 0.0% |

## Reading the profile

Written by hand from the tables above; `make bench-ordrar` keeps this section and rewrites the rest.

- **The clock stops at the first board complete, not at the first thing drawn.** The cold time ends at the first complete frame where the first board of the design's manifest is read, all in view at full detail, so with its text as text, with no image missing; each run then clicks on it and fails unless a node is selected. A design viewer page used to open fitted to all its boards, which on Omslag (3038 by 62521 px) is a speck of about 21 by 10 px at zoom 0.013 with its text as bars. A page whose boards would be drawn at reduced detail now opens fitted to its first board (`Scene::opening`), a link to a node moves the view to that node's board, and pages that can be read whole open as before.
- **The cold load is still mostly the network, but a much shorter one.** The longest request of the load is the design's outline (`GET .../docs/:doc/html?view=outline`), which the viewer needs before it can show anything. It was about 1 MB and 800 ms. The boxes of the placeholders (`crates/omni-api/src/placeholders.rs`) are laid out in the background when a design is written and stored in RustFS, one object per design system version, so that the first outline after a restart reads them instead of laying out about 550 of the 797 boards (the API's part of the request went from about 500 ms to about 120 ms). A box that is not stored yet, or was laid out against an older design system, is laid out when the outline is read, as before. And the proxy compresses API responses (`encode zstd gzip` in `docker/Caddyfile`), which takes the outline from about 1 MB to about 110 KB. The first run after the stack starts is slower, and a median can hide that, so compare the runs too. The cold time depends a lot on how busy the machine is: the requests of this baseline start and take about twice as long as those of the run before it (the outline starts at 340 ms against 154 ms), which adds up to 1042 ms against 560 ms, while the previews of the first page add a request that runs next to the images (about 80 ms) and do not lengthen the load by themselves. After the outline, the viewer waits for the images of the page before it shows it, then reads the boards in view. Parsing, layout, painting and the GPU together are about a quarter of the load.
- **A board that arrives is put in place, not the page shown again.** The first measurement had the viewer show the whole page again each time boards arrived, and each show held the main thread for about 0.5 s (the page laid out twice, and the layers panel listed again); that was the 95th percentile, about 700 ms. Now the page is shown once, with every board a placeholder, and a board that is read goes to `_omni_render_scene_update_boards` and is put in place in stages (`_omni_render_scene_update_step`): its instances and tokens are resolved about 2 ms at a time, then it is laid out and indexed, and the view, the other boards' pictures and the selection stay. The viewer does one stage per animation frame, or a few when they are quick, and none late in a frame that a heavy draw has used. The layers panel lists the new boards once the view and the arrivals have been still for 300 ms, and a comment pin does not wait for that: it comes with its board. Showing the page again with the same design system no longer lays the loaded page out for nothing, and the boards in view are all read, not only the first 48.
- **A board in view shows its content before it is read.** Reading boards on demand can't keep up with a view: zooming out once brings about 150 boards in view at the viewer's pace of six reads at a time, and the frame times alone would look just as good with grey boxes on screen. So the API makes a preview of each board it stores (`crates/omni-api/src/previews.rs`, `view::board_preview`): 1/16 of the board at reduced detail, about 1 KB as a PNG, kept in RustFS by chunk and design system version, made in the background of a write, or when it is read if it is not there yet. The viewer reads the previews of a page before it shows the page (`?view=previews`, about 270 ms for the 769 boards here) and draws a board from its preview at every zoom until the board is read and put in place. This costs a frame nothing it did not cost already: a preview is one image, like a snapshot. A preview is drawn in the design system's first mode; the board takes the chosen mode when it arrives.
- **Frames come at the pace of the display.** The median frame is 8.3 ms, the interval of the animation frames of this display, and the 95th percentile is about 11 ms. Two things used to make frames uneven. `visibleBoards` drew a frame when one was waiting, outside the animation frames, which left the next animation frame with nothing to draw and doubled that interval; it now draws only a page that has not been drawn yet, to fit its view. And `drawFrame` read the canvas's size from the page every frame, which forces a layout of the page; the resize observer keeps it now. The pins and the selection outline are drawn in the same frame as the board (the pins render synchronously from the view listener), so none trails it.
- **Cached pictures keep the draw short.** A board far out is drawn from a snapshot, which a viewer makes no larger than the view can show (a power of two from 1/64 to 1/2 of the board) and makes again, sharper, when the view comes closer. A frame spends about 3 ms on making pictures and snapshots; a board that has an older or smaller one (its preview, or a coarser snapshot) is drawn from that and made sharp in the next frames, and a board with none is made at once, so no board in view is blank. Painting is about 7% of a frame and uploading to the GPU about 4%.
- **Memory is within the limit with room to spare.** The API's peak is the highest sample of the container's memory as `docker stats` reports it, about once a second, from before the navigation to the end of the path, after a restart, so the import is not in it. The figure is the highest peak of the five runs, not their median. A spike shorter than a second can fall between two samples. It was about 250 MiB when the outline laid out the placeholders' boxes itself. The import itself leaves the API near 2.5 GB resident, which the restart discards.
- **The path.** The file's 797 boards are spread over five pages and the viewer shows one page at a time, so the pan and zoom runs on the page with 769 boards; the file opens on the first page, of 5 boards, whose first board is the one of the cold measurement. That page cannot have all its boards in view: it is 8118 by 329640 px, which would need a zoom of about 0.0024, and the viewer stops at 0.01 (`MIN_ZOOM`). So the path zooms out to 0.01 and pans the whole page from there, with about 350 of the 769 boards in view at once. The "most boards in view" of a run depends on where its few frames land, so it differs between runs.
- **Limits of the method.** A frame's time is the time from the previous complete frame in the running viewer, so unlike renderer gate A, which draws frames back to back, it includes the wait for the next animation frame. Layout is the renderer's and includes resolving instances and tokens, because it does both in one step; the browser's page layout is in other. Handling the wheel events counts as painting. Uploading to the GPU is Skia submitting its commands plus the wait for `gl.finish()`; the GPU's own execution time is not separated from it. In the cold table, script that runs in a task shorter than 50 ms while a request is open counts as network, because only the renderer's calls and long tasks are seen. The renderer's phase times come from `_omni_render_scene_timings`, added for this bench.
