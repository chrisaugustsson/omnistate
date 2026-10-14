# Penpot comparison

`omni-penpot compare <file.penpot> --out <dir>` converts the boards of a Penpot
export (as `omni-penpot convert` does) and sets OmniState's rendering of each
board beside Penpot's own. It measures; it does not pass or fail. What to fix is
decided after reading the result.

## What it writes

`<dir>/compare.json` holds `tolerance` and `boards`, one entry per converted
board in the order of `convert`. Each entry has `board`, `boardId`, `page`,
`layoutOnly`, `note`, `pixel`, `layout` and `unconverted`. The images that
`pixel` names are in `<dir>/images/`, with paths relative to `<dir>`. The same
inputs always give the same files.

## Pixel comparison

Penpot's PNG of a board is looked for in the folder beside the export that has
the export's name without `.penpot` (`fixtures/penpot/tirebud-idag/` for
`tirebud-idag.penpot`). Penpot names the PNG after the board, with each ` / `
written ` _ `, and numbers boards of the same name `-2`, `-3` and so on, the
last one in the stacking order keeping the bare name. A PNG that no board has,
such as one of a text on the page, is ignored. Twins are counted among boards
only, not among the loose shapes on the page.

The board is drawn at 2x with its top left corner at the image's corner, on a
canvas of the size of Penpot's PNG (which can be a pixel more than twice the
board where the board has a fraction). `render` is that image.

**Tolerance: 8.** A pixel is the same when its red, green, blue and alpha are each
at most 8 steps (of 255) from Penpot's, and differs when any is more. This lets
through the rounding differences of two renderers and nothing a person would
call a different color. The number is `tolerance` in `compare.json` and
`TOLERANCE` in `crates/omni-penpot/src/compare.rs`; change both together with
this text.

`pixel` has `width`, `height`, `compared`, `masked`, `differing`, `share`
(`differing / compared`), `render` and `diff`. The `diff` image shows Penpot's
PNG faded, every differing pixel in magenta (255, 0, 255), and the masked boxes
in flat light blue.

Expect many differing pixels around text: Penpot draws text through the browser
and OmniState through Skia, with the bundled Inter standing in for fonts that
are not supplied, so glyphs and line breaks differ slightly. Differences in
solid areas, edges, shadows or colors are the ones to look at.

## Masked shapes

A shape the converter left out whole (a report entry with no property, such as a
boolean shape) is listed in `unconverted` with its id, name and reason, and its
box in Penpot's PNG is masked: those pixels are in `masked`, not in `compared`,
and are never highlighted. A shape converted without one property (a blur, say)
is drawn and compared, and is not masked.

## Layout comparison

For each converted shape of a board, the board included, Penpot's stored box
(page coordinates) is set beside the box OmniState's layout computes. `compared`
counts the shapes; `mismatches` lists those more than 2 px off in x, y, width or
height, furthest first, with both boxes and `offBy`, the largest of the four
differences. Hidden shapes, and shapes inside hidden ones, are not compared:
nothing is drawn for them.

A layout mismatch means an element is in another place or has another size. A
board that matches in layout but differs in pixels has a drawing difference.

## Boards without a PNG

A board with no Penpot PNG, which is every board of `tirebud-ordrar.penpot`,
gets the layout comparison only. Its entry has `layoutOnly` true, `pixel` null
and a `note` that says "Layout comparison only" and why.

## The report page

`make penpot-compare` converts and compares every `.penpot` file directly in
`fixtures/penpot/` (`PENPOT_FIXTURES` replaces the folder) and writes
`docs/penpot-compare/index.html` (`PENPOT_REPORT` replaces the folder) with the
images it shows in `images/` beside it. It is the same as
`omni-penpot report <folder> --out <dir>`. The page works from disk: every image
path is relative, and Penpot's PNGs are copied into the report folder. The
`images/` folder is emptied before each run. The folder is gitignored, like the
fixtures, because it holds Penpot's designs.

At the top, the summary gives the number of boards, the median pixel score (the
middle score of the boards that have a PNG, the mean of the two middle ones for
an even number) and the totals of layout mismatches and unconverted shapes.
Below it, the boards are sorted worst first: the boards that have a pixel score
come first, the largest share of differing pixels first, and then the boards
without a PNG, the most layout mismatches first. Ties keep the order of the
files and the boards.

Each board shows Penpot's PNG, OmniState's render and the difference image side
by side (click one to open it full size), its pixel score as a percentage (lower
is better, 0.0% is a perfect match), the layout mismatches with both boxes and
how far off they are, and the unconverted shapes with the reason. A board with
no PNG shows no images and says "Layout comparison only". Names and reasons come
from the design and are escaped in the page.
