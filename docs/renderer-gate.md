# Renderer gate

The pass/fail tests of the renderer (Q35). Each section is written by the script of its gate.

## Gate A: performance

Result: **PASS**

- Date: 2026-10-14
- Machine: Apple M4 Pro, 24 GB memory, Darwin 25.6.0 (arm64), Chrome 155.0.8059.41
- WebGL renderer: `ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version)`
- Chrome: headed, driven by Playwright, 1440x900 viewport, device pixel ratio 2
- Method: 5 runs, each measurement from a cold page load (a new browser, with an empty cache); the verdict is the median of the 5 runs
- Times are in milliseconds. A frame is complete when the GPU has finished it (`gl.finish()`).

### Medians and limits

| Measurement | Median | Limit | Verdict |
| --- | --- | --- | --- |
| 2000-node board, data loaded to first complete frame | 367.2 | under 500 | PASS |
| 100-board file, data loaded to first frame of every board | 1098.8 | under 2000 | PASS |
| Zoom and pan path, 95th-percentile frame time | 8.8 | 16.7 or less | PASS |

### The 5 runs

| Run | Board | File | Frame p95 |
| --- | --- | --- | --- |
| 1 | 433.4 | 1137.6 | 8.4 |
| 2 | 370.1 | 1117.9 | 9.1 |
| 3 | 365.0 | 1086.4 | 8.4 |
| 4 | 367.2 | 1098.8 | 9.3 |
| 5 | 366.0 | 1095.1 | 8.8 |

## Gate B: layout parity

Result: **PASS**

- Date: 2026-10-14
- Machine: Apple M4 Pro, 24 GB memory, Darwin 25.6.0 (arm64)
- Chrome: Chrome 155.0.8059.41, headed, driven by Playwright, 1440x900 viewport, device pixel ratio 2
- Compared: every node of the stress file, the box that `getBoundingClientRect` gives in Chrome against the box that the WebAssembly build of the renderer lays out in the page
- Chrome renders the export of `make chrome-boxes`: line heights in px, never `normal`, and the bundled Inter loaded through `@font-face`
- Tolerance: 2 px, for each of x, y, width and height
- Nodes compared: 36629
- Mismatches: 0
- Largest difference: 0.055 px, the x of node t3n3q94x
- Known differences of docs/formats/layout.md: no node of the stress file is beyond 2 px, so each is either fixed or stays within 2 px on the stress file
