//! Writes the stress file: a large, realistic design file that every renderer
//! story is tested on. Run it with `make stress-file [SEED=<n>] [OUT=<path>]`.
//!
//! The file is one page whose children are 100 boards, laid out as a grid on
//! the page. One board is text-heavy and has exactly 2,000 nodes (the board
//! and everything inside it); the other 99 hold 300 to 400 nodes each, so the
//! whole file has at least 30,000 nodes. The boards use every node type except
//! instances, every layout mode, and literal values only, so tokens and
//! instances stay with US-032. The same seed always gives the same bytes.

use omni_core::design::Node;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::{env, fs};

const DEFAULT_SEED: u64 = 0x0D15_EA5E;
const DEFAULT_OUT: &str = "render-sandbox/build/stress-file.json";

const BOARDS: usize = 100;
const HEAVY_BOARD_NODES: usize = 2_000;
const BOARD_NODES: std::ops::RangeInclusive<usize> = 300..=400;
const BOARDS_PER_ROW: usize = 10;
const BOARD_GAP: f64 = 120.0;

/// The alphabet of node IDs: Crockford's base32 in lowercase.
const ID_ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
const HEX: &[u8] = b"0123456789abcdef";

const WORDS: &[&str] = &[
    "design",
    "board",
    "layout",
    "render",
    "canvas",
    "frame",
    "stack",
    "token",
    "color",
    "radius",
    "spacing",
    "type",
    "scale",
    "grid",
    "column",
    "row",
    "align",
    "gap",
    "padding",
    "surface",
    "primary",
    "accent",
    "muted",
    "border",
    "shadow",
    "hover",
    "focus",
    "active",
    "disabled",
    "button",
    "input",
    "label",
    "card",
    "list",
    "table",
    "header",
    "footer",
    "sidebar",
    "modal",
    "toast",
    "avatar",
    "badge",
    "chip",
    "tab",
    "menu",
    "tooltip",
    "form",
    "field",
    "search",
    "filter",
    "sort",
    "export",
    "import",
    "share",
    "comment",
    "review",
    "approve",
    "version",
    "draft",
    "merge",
    "branch",
    "project",
    "workspace",
    "agent",
    "thread",
    "decision",
    "update",
];
const COLORS: &[&str] = &[
    "#0f172a", "#1e293b", "#475569", "#94a3b8", "#e2e8f0", "#f8fafc", "#2563eb", "#7c3aed",
    "#db2777", "#dc2626", "#ea580c", "#ca8a04", "#16a34a", "#0d9488", "#ffffff", "#000000",
];
const FONT_SIZES: &[u32] = &[12, 14, 16, 18, 20, 24, 32];
const LINE_HEIGHTS: &[f64] = &[1.25, 1.5];
const FONT_WEIGHTS: &[u32] = &[400, 500, 600, 700];

/// A small deterministic random generator (SplitMix64), so that a seed gives
/// the same file on every machine without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number from `low` to `high`, both included.
    fn range(&mut self, low: usize, high: usize) -> usize {
        low + (self.next() % (high - low + 1) as u64) as usize
    }

    /// A length in whole pixels from `low` to `high`, both included.
    fn px(&mut self, low: usize, high: usize) -> Value {
        json!(self.range(low, high))
    }

    /// Whether an event with a chance of `percent` out of 100 happens.
    fn chance(&mut self, percent: usize) -> bool {
        self.range(0, 99) < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.range(0, items.len() - 1)]
    }
}

/// How a container places its children.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    None,
    Flex,
    Grid,
}

/// What the children of one container are placed in.
#[derive(Clone, Copy)]
struct Parent {
    mode: Mode,
    columns: usize,
}

/// How one board is filled.
#[derive(Clone, Copy)]
struct Recipe {
    /// Chance in percent that a child is a frame rather than a leaf.
    frame: usize,
    /// Chance in percent that a leaf is a text.
    text: usize,
    /// The most nodes inside one frame.
    frame_nodes: usize,
    max_depth: usize,
}

const TEXT_HEAVY: Recipe = Recipe {
    frame: 10,
    text: 85,
    frame_nodes: 40,
    max_depth: 5,
};
const MIXED: Recipe = Recipe {
    frame: 20,
    text: 35,
    frame_nodes: 25,
    max_depth: 6,
};

struct Generator {
    rng: Rng,
    ids: HashSet<String>,
    attachments: Vec<String>,
}

fn insert(object: &mut Map<String, Value>, key: &str, value: Value) {
    object.insert(key.to_string(), value);
}

impl Generator {
    fn new(seed: u64) -> Self {
        let mut generator = Generator {
            rng: Rng(seed),
            ids: HashSet::new(),
            attachments: Vec::new(),
        };
        generator.attachments = (0..24).map(|_| generator.uuid()).collect();
        generator
    }

    fn id(&mut self) -> String {
        loop {
            let id: String = (0..8)
                .map(|_| *self.rng.pick(ID_ALPHABET) as char)
                .collect();
            if self.ids.insert(id.clone()) {
                return id;
            }
        }
    }

    fn uuid(&mut self) -> String {
        let mut digits =
            |count: usize| -> String { (0..count).map(|_| *self.rng.pick(HEX) as char).collect() };
        format!(
            "{}-{}-{}-{}-{}",
            digits(8),
            digits(4),
            digits(4),
            digits(4),
            digits(12)
        )
    }

    fn attachment(&mut self) -> Value {
        let id = self.rng.pick(&self.attachments.clone()).clone();
        json!(format!("attachment:{id}"))
    }

    fn color(&mut self) -> Value {
        let color = *self.rng.pick(COLORS);
        if self.rng.chance(10) {
            json!(format!("{color}{:02x}", self.rng.range(0x40, 0xcc)))
        } else {
            json!(color)
        }
    }

    fn words(&mut self, count: usize) -> String {
        let words: Vec<&str> = (0..count).map(|_| *self.rng.pick(WORDS)).collect();
        let mut text = words.join(" ");
        if let Some(first) = text.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        text
    }

    /// A node named by a couple of random words.
    fn named(&mut self, kind: &str) -> Map<String, Value> {
        let name = self.words(2);
        self.node(kind, name)
    }

    fn node(&mut self, kind: &str, name: String) -> Map<String, Value> {
        let mut node = Map::new();
        insert(&mut node, "id", json!(self.id()));
        insert(&mut node, "type", json!(kind));
        insert(&mut node, "name", json!(name));
        node
    }

    fn solid(&mut self) -> Value {
        json!({"type": "solid", "color": self.color()})
    }

    fn fills(&mut self) -> Value {
        let mut fills = vec![self.solid()];
        if self.rng.chance(15) {
            let (from, to) = (self.color(), self.color());
            fills.push(json!({
                "type": "linear-gradient",
                "angle": self.rng.range(0, 359),
                "stops": [
                    {"position": 0, "color": from},
                    {"position": 1, "color": to},
                ],
            }));
        }
        if self.rng.chance(8) {
            let image = self.attachment();
            let fit = *self.rng.pick(&["cover", "contain"]);
            fills.push(json!({"type": "image", "image": image, "fit": fit}));
        }
        Value::Array(fills)
    }

    fn strokes(&mut self) -> Value {
        json!([{"color": self.color(), "width": self.rng.range(1, 3)}])
    }

    fn radius(&mut self) -> Value {
        if self.rng.chance(70) {
            json!({
                "topLeft": self.rng.range(2, 16),
                "topRight": self.rng.range(2, 16),
                "bottomRight": self.rng.range(2, 16),
                "bottomLeft": self.rng.range(2, 16),
            })
        } else {
            json!({"topLeft": self.rng.range(2, 24), "bottomRight": self.rng.range(2, 24)})
        }
    }

    fn shadows(&mut self) -> Value {
        let kind = *self.rng.pick(&["drop", "drop", "inner"]);
        let mut shadow = json!({
            "type": kind,
            "color": self.color(),
            "offsetX": self.rng.range(0, 6),
            "offsetY": self.rng.range(1, 12),
        });
        if self.rng.chance(80) {
            insert(
                shadow.as_object_mut().expect("an object"),
                "blur",
                self.rng.px(2, 24),
            );
        }
        json!([shadow])
    }

    /// Style properties that frames, images and vectors share, each with its
    /// own chance, and the opacity of any node.
    fn decorate(&mut self, node: &mut Map<String, Value>, chances: [usize; 4]) {
        let [stroke, radius, shadow, opacity] = chances;
        if self.rng.chance(stroke) {
            insert(node, "strokes", self.strokes());
        }
        if self.rng.chance(radius) {
            insert(node, "radius", self.radius());
        }
        if self.rng.chance(shadow) {
            insert(node, "shadows", self.shadows());
        }
        if self.rng.chance(opacity) {
            let opacity = self.rng.range(30, 95) as f64 / 100.0;
            insert(node, "opacity", json!(opacity));
        }
    }

    /// Where a child sits in its parent, and whether it is out of the flow
    /// (and so placed at `x` and `y`).
    fn place(&mut self, node: &mut Map<String, Value>, parent: Parent) -> bool {
        let absolute = parent.mode != Mode::None && self.rng.chance(3);
        if absolute {
            insert(node, "absolute", json!(true));
        }
        let positioned = absolute || parent.mode == Mode::None;
        if positioned {
            insert(node, "x", self.rng.px(0, 480));
            insert(node, "y", self.rng.px(0, 640));
        } else if parent.mode == Mode::Grid {
            if parent.columns >= 2 && self.rng.chance(20) {
                let start = self.rng.range(1, parent.columns);
                let span = self.rng.range(1, parent.columns - start + 1);
                insert(node, "gridColumn", json!({"start": start, "span": span}));
            }
            if self.rng.chance(10) {
                insert(node, "gridRow", json!({"span": self.rng.range(1, 2)}));
            }
        }
        positioned
    }

    fn leaf(&mut self, recipe: Recipe, parent: Parent) -> Value {
        if self.rng.chance(recipe.text) {
            self.text(parent)
        } else if self.rng.chance(25) {
            self.image(parent)
        } else {
            self.vector(parent)
        }
    }

    fn text(&mut self, parent: Parent) -> Value {
        let content = if self.rng.chance(15) {
            // Several paragraphs, to wrap onto many lines.
            let paragraphs: Vec<String> = (0..self.rng.range(2, 3))
                .map(|_| {
                    let count = self.rng.range(12, 40);
                    self.words(count)
                })
                .collect();
            paragraphs.join("\n")
        } else {
            let count = self.rng.range(1, 12);
            self.words(count)
        };
        let mut node = self.named("text");
        let positioned = self.place(&mut node, parent);
        let width = match parent.mode {
            _ if positioned => json!({"fixed": self.rng.range(80, 360)}),
            Mode::Grid if self.rng.chance(70) => json!("fill"),
            Mode::Flex if self.rng.chance(25) => json!("fill"),
            Mode::Flex if self.rng.chance(25) => json!("hug"),
            _ => json!({"fixed": self.rng.range(80, 360)}),
        };
        insert(&mut node, "width", width);
        insert(&mut node, "height", json!("hug"));
        if self.rng.chance(5) {
            insert(&mut node, "maxWidth", self.rng.px(240, 480));
        }
        insert(&mut node, "content", json!(content));
        insert(&mut node, "fontFamily", json!("Inter"));
        insert(&mut node, "fontSize", json!(*self.rng.pick(FONT_SIZES)));
        insert(&mut node, "lineHeight", json!(*self.rng.pick(LINE_HEIGHTS)));
        if self.rng.chance(40) {
            insert(&mut node, "fontWeight", json!(*self.rng.pick(FONT_WEIGHTS)));
        }
        if self.rng.chance(10) {
            insert(&mut node, "fontStyle", json!("italic"));
        }
        if self.rng.chance(10) {
            insert(&mut node, "letterSpacing", json!(self.rng.range(0, 2)));
        }
        if self.rng.chance(30) {
            let align = *self.rng.pick(&["left", "center", "right", "justify"]);
            insert(&mut node, "textAlign", json!(align));
        }
        if self.rng.chance(8) {
            let decoration = *self.rng.pick(&["underline", "line-through", "none"]);
            insert(&mut node, "textDecoration", json!(decoration));
        }
        if self.rng.chance(70) {
            insert(&mut node, "color", self.color());
        }
        if self.rng.chance(5) {
            insert(
                &mut node,
                "opacity",
                json!(self.rng.range(40, 90) as f64 / 100.0),
            );
        }
        Value::Object(node)
    }

    fn image(&mut self, parent: Parent) -> Value {
        let mut node = self.named("image");
        let positioned = self.place(&mut node, parent);
        let width = if !positioned && parent.mode != Mode::None && self.rng.chance(25) {
            json!("fill")
        } else {
            json!({"fixed": self.rng.range(48, 320)})
        };
        insert(&mut node, "width", width);
        insert(
            &mut node,
            "height",
            json!({"fixed": self.rng.range(48, 240)}),
        );
        insert(&mut node, "image", self.attachment());
        insert(
            &mut node,
            "fit",
            json!(*self.rng.pick(&["cover", "contain"])),
        );
        self.decorate(&mut node, [10, 40, 15, 5]);
        Value::Object(node)
    }

    fn vector(&mut self, parent: Parent) -> Value {
        let mut node = self.named("vector");
        self.place(&mut node, parent);
        let side = self.rng.range(16, 64);
        insert(&mut node, "width", json!({"fixed": side}));
        insert(&mut node, "height", json!({"fixed": side}));
        insert(&mut node, "path", json!(self.path(side)));
        if self.rng.chance(85) {
            insert(&mut node, "fills", self.fills_without_images());
        }
        if self.rng.chance(30) {
            insert(&mut node, "strokes", self.strokes());
        }
        if self.rng.chance(10) {
            insert(&mut node, "shadows", self.shadows());
        }
        if self.rng.chance(5) {
            insert(
                &mut node,
                "opacity",
                json!(self.rng.range(40, 90) as f64 / 100.0),
            );
        }
        Value::Object(node)
    }

    fn fills_without_images(&mut self) -> Value {
        let fill = self.solid();
        json!([fill])
    }

    /// SVG path data inside a square of `side` pixels.
    fn path(&mut self, side: usize) -> String {
        let point = |rng: &mut Rng| format!("{} {}", rng.range(0, side), rng.range(0, side));
        let mut path = format!("M{}", point(&mut self.rng));
        for _ in 0..self.rng.range(2, 5) {
            match self.rng.range(0, 2) {
                0 => path += &format!(" L{}", point(&mut self.rng)),
                1 => {
                    let (a, b, c) = (
                        point(&mut self.rng),
                        point(&mut self.rng),
                        point(&mut self.rng),
                    );
                    path += &format!(" C{a} {b} {c}");
                }
                _ => path += &format!(" Q{} {}", point(&mut self.rng), point(&mut self.rng)),
            }
        }
        path.push_str(" Z");
        path
    }

    fn layout(&mut self) -> (Value, Parent) {
        let mode = match self.rng.range(0, 99) {
            0..=29 => Mode::None,
            30..=79 => Mode::Flex,
            _ => Mode::Grid,
        };
        let mut layout = Map::new();
        let mut columns = 1;
        match mode {
            Mode::None => insert(&mut layout, "mode", json!("none")),
            Mode::Flex => {
                insert(&mut layout, "mode", json!("flex"));
                let directions = ["row", "column", "row-reverse", "column-reverse"];
                if self.rng.chance(80) {
                    insert(&mut layout, "direction", json!(*self.rng.pick(&directions)));
                }
                if self.rng.chance(40) {
                    let wrap = *self.rng.pick(&["wrap", "wrap", "wrap-reverse", "nowrap"]);
                    insert(&mut layout, "wrap", json!(wrap));
                }
                if self.rng.chance(60) {
                    let align = ["start", "center", "end", "stretch", "baseline"];
                    insert(&mut layout, "alignItems", json!(*self.rng.pick(&align)));
                }
                if self.rng.chance(40) {
                    let justify = [
                        "start",
                        "center",
                        "end",
                        "space-between",
                        "space-around",
                        "space-evenly",
                    ];
                    insert(
                        &mut layout,
                        "justifyContent",
                        json!(*self.rng.pick(&justify)),
                    );
                }
            }
            Mode::Grid => {
                insert(&mut layout, "mode", json!("grid"));
                columns = self.rng.range(2, 4);
                let tracks: Vec<Value> = (0..columns).map(|_| self.track()).collect();
                insert(&mut layout, "columns", Value::Array(tracks));
                if self.rng.chance(30) {
                    let rows: Vec<Value> =
                        (0..self.rng.range(1, 3)).map(|_| self.track()).collect();
                    insert(&mut layout, "rows", Value::Array(rows));
                }
            }
        }
        if mode != Mode::None {
            if self.rng.chance(80) {
                let gap = if self.rng.chance(50) {
                    json!({"row": self.rng.range(0, 24), "column": self.rng.range(0, 24)})
                } else {
                    json!({"row": self.rng.range(0, 24)})
                };
                insert(&mut layout, "gap", gap);
            }
            if self.rng.chance(70) {
                let padding = if self.rng.chance(50) {
                    let all = self.rng.range(4, 32);
                    json!({"top": all, "right": all, "bottom": all, "left": all})
                } else {
                    json!({"top": self.rng.range(4, 24), "left": self.rng.range(4, 24)})
                };
                insert(&mut layout, "padding", padding);
            }
        }
        (Value::Object(layout), Parent { mode, columns })
    }

    fn track(&mut self) -> Value {
        match self.rng.range(0, 2) {
            0 => json!({"fr": self.rng.range(1, 3)}),
            1 => json!({"fixed": self.rng.range(80, 240)}),
            _ => json!("auto"),
        }
    }

    /// `count` children, in nodes counted with everything inside them.
    fn children(
        &mut self,
        mut count: usize,
        depth: usize,
        recipe: Recipe,
        parent: Parent,
    ) -> Value {
        let mut children = Vec::new();
        while count > 0 {
            if count >= 3 && depth < recipe.max_depth && self.rng.chance(recipe.frame) {
                let inside = self.rng.range(2, (count - 1).min(recipe.frame_nodes));
                count -= 1 + inside;
                children.push(self.frame(inside, depth + 1, recipe, parent));
            } else {
                count -= 1;
                children.push(self.leaf(recipe, parent));
            }
        }
        Value::Array(children)
    }

    /// A frame with `inside` nodes in it.
    fn frame(&mut self, inside: usize, depth: usize, recipe: Recipe, parent: Parent) -> Value {
        let mut node = self.named("frame");
        let positioned = self.place(&mut node, parent);
        if self.rng.chance(1) {
            insert(&mut node, "visible", json!(false));
        }
        let (layout, own) = self.layout();
        // A frame that hugs its height needs flow children to measure.
        let height_hugs = own.mode != Mode::None && self.rng.chance(70);
        let width = match parent.mode {
            _ if positioned || own.mode == Mode::None => json!({"fixed": self.rng.range(160, 420)}),
            Mode::Grid if self.rng.chance(60) => json!("fill"),
            Mode::Flex if self.rng.chance(40) => json!("fill"),
            Mode::Flex if self.rng.chance(20) => json!("hug"),
            _ => json!({"fixed": self.rng.range(160, 420)}),
        };
        insert(&mut node, "width", width);
        let height = if height_hugs {
            json!("hug")
        } else {
            json!({"fixed": self.rng.range(80, 320)})
        };
        insert(&mut node, "height", height);
        insert(&mut node, "layout", layout);
        if self.rng.chance(60) {
            insert(&mut node, "fills", self.fills());
        }
        self.decorate(&mut node, [20, 40, 15, 5]);
        if self.rng.chance(15) {
            insert(&mut node, "clip", json!(true));
        }
        insert(
            &mut node,
            "children",
            self.children(inside, depth, recipe, own),
        );
        Value::Object(node)
    }

    /// A board of exactly `nodes` nodes, the board included.
    fn board(
        &mut self,
        index: usize,
        nodes: usize,
        recipe: Recipe,
    ) -> (Map<String, Value>, f64, f64) {
        let mut node = self.node("board", format!("Board {}", index + 1));
        let (layout, own) = self.layout();
        let width = self.rng.range(640, 960) as f64;
        let height = if nodes == HEAVY_BOARD_NODES {
            2_400.0
        } else {
            self.rng.range(700, 1_400) as f64
        };
        insert(&mut node, "width", json!({"fixed": width}));
        insert(&mut node, "height", json!({"fixed": height}));
        insert(&mut node, "layout", layout);
        insert(
            &mut node,
            "fills",
            json!([{"type": "solid", "color": "#ffffff"}]),
        );
        insert(&mut node, "clip", json!(true));
        insert(
            &mut node,
            "children",
            self.children(nodes - 1, 1, recipe, own),
        );
        (node, width, height)
    }

    fn page(&mut self) -> Value {
        let heavy = self.rng.range(0, BOARDS - 1);
        let mut boards = Vec::new();
        for index in 0..BOARDS {
            boards.push(if index == heavy {
                self.board(index, HEAVY_BOARD_NODES, TEXT_HEAVY)
            } else {
                let nodes = self.rng.range(*BOARD_NODES.start(), *BOARD_NODES.end());
                self.board(index, nodes, MIXED)
            });
        }
        // Rows of boards, each as tall as its tallest board.
        let mut y = 0.0;
        let mut placed = Vec::new();
        for row in boards.chunks_mut(BOARDS_PER_ROW) {
            let mut x = 0.0;
            for (board, width, _) in row.iter_mut() {
                insert(board, "x", json!(x));
                insert(board, "y", json!(y));
                x += *width + BOARD_GAP;
            }
            y += row.iter().map(|(_, _, height)| *height).fold(0.0, f64::max) + BOARD_GAP;
            placed.extend(row.iter().map(|(board, _, _)| Value::Object(board.clone())));
        }
        let mut page = self.node("page", "Stress file".to_string());
        insert(&mut page, "children", Value::Array(placed));
        Value::Object(page)
    }
}

struct Options {
    seed: u64,
    out: PathBuf,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        seed: DEFAULT_SEED,
        out: PathBuf::from(DEFAULT_OUT),
    };
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--seed" => {
                let seed = value()?;
                options.seed = seed
                    .parse()
                    .map_err(|_| format!("`{seed}` is not a seed: use a whole number"))?;
            }
            "--out" => options.out = PathBuf::from(value()?),
            _ => {
                return Err(format!(
                    "unknown option {flag}: use --seed <n> and --out <path>"
                ));
            }
        }
    }
    Ok(options)
}

fn write(options: &Options) -> Result<usize, String> {
    let page = Generator::new(options.seed).page();
    // Reading the page back through the design model checks the file the way
    // every reader will, and writes it in the model's own canonical form.
    let page: Node = serde_json::from_value(page)
        .map_err(|error| format!("the generated file is not a valid design: {error}"))?;
    let mut json = serde_json::to_string(&page).map_err(|error| error.to_string())?;
    json.push('\n');
    if let Some(directory) = options.out.parent() {
        fs::create_dir_all(directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
    }
    fs::write(&options.out, json).map_err(|error| format!("{}: {error}", options.out.display()))?;
    Ok(count(&page))
}

fn count(node: &Node) -> usize {
    1 + node.children().iter().map(count).sum::<usize>()
}

fn main() -> ExitCode {
    let result = options().and_then(|options| {
        let nodes = write(&options)?;
        println!(
            "wrote {nodes} nodes in {BOARDS} boards to {} (seed {})",
            options.out.display(),
            options.seed
        );
        Ok(())
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("stress-file: {message}");
            ExitCode::FAILURE
        }
    }
}
