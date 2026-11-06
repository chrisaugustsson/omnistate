//! The prototype graph of one page of a design (US-22): its flows, the
//! interactions of every board and what they point at, read from the page's
//! outline entry and the boards the board route answered with.

use crate::json::Value;
use std::collections::HashMap;

const DEFAULT_DURATION_MS: &str = "300";

struct Board {
    id: String,
    name: String,
}

struct Flow {
    name: String,
    start: String,
}

enum Action {
    /// Navigate, open overlay and swap overlay point at a board.
    Board {
        kind: String,
        dest: String,
    },
    /// Scroll to points at a layer of the board it is on.
    Layer {
        dest: String,
        missing: bool,
    },
    /// Any other action, as it is printed.
    Plain(String),
    Conditional(Vec<Action>),
}

struct Interaction {
    board: usize,
    layer: String,
    layer_id: String,
    path: String,
    trigger: String,
    actions: Vec<Action>,
    style: String,
}

pub struct Graph {
    page_name: String,
    page_id: String,
    flows: Vec<Flow>,
    boards: Vec<Board>,
    interactions: Vec<Interaction>,
    /// The name of every board of the doc, by ID.
    names: HashMap<String, String>,
}

impl Graph {
    /// The graph of the page `page` of the outline, with `boards` the board
    /// nodes (a board and its subtree as JSON) of that page in any order.
    pub fn from(page: &Value, boards: &[Value]) -> Graph {
        let children = page
            .get("children")
            .and_then(Value::as_array)
            .unwrap_or_default();
        let listed: Vec<Board> = children
            .iter()
            .map(|board| Board {
                id: text(board, "id"),
                name: text(board, "name"),
            })
            .collect();
        let names = listed
            .iter()
            .map(|board| (board.id.clone(), board.name.clone()))
            .collect();
        let flows = page
            .get("flows")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .map(|flow| Flow {
                name: text(flow, "name"),
                start: text(flow, "start"),
            })
            .collect();
        let mut graph = Graph {
            page_name: text(page, "name"),
            page_id: text(page, "id"),
            flows,
            boards: listed,
            interactions: Vec::new(),
            names,
        };
        graph.read_boards(boards);
        graph
    }

    /// Registers the boards of every page of the doc, so that an interaction
    /// pointing at a board of another page is not dangling.
    pub fn in_doc(mut self, pages: &[Value]) -> Graph {
        for board in pages.iter().flat_map(|page| {
            page.get("children")
                .and_then(Value::as_array)
                .unwrap_or_default()
        }) {
            self.names.insert(text(board, "id"), text(board, "name"));
        }
        self
    }

    fn read_boards(&mut self, boards: &[Value]) {
        for node in boards {
            let id = text(node, "id");
            let Some(index) = self.boards.iter().position(|board| board.id == id) else {
                continue;
            };
            let mut layers = Vec::new();
            let mut found = Vec::new();
            collect(node, "", &mut layers, &mut found);
            for (layer, layer_id, path, interactions) in found {
                for interaction in interactions {
                    self.interactions.push(Interaction {
                        board: index,
                        layer: layer.clone(),
                        layer_id: layer_id.clone(),
                        path: path.clone(),
                        trigger: trigger_text(interaction.get("trigger")),
                        actions: Self::actions_of(interaction, &layers),
                        style: style_text(interaction),
                    });
                }
            }
        }
    }

    fn actions_of(interaction: &Value, layers: &[String]) -> Vec<Action> {
        interaction
            .get("actions")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .map(|action| Self::action(action, layers))
            .collect()
    }

    fn action(action: &Value, layers: &[String]) -> Action {
        let kind = text(action, "type");
        let dest = text(action, "dest");
        match kind.as_str() {
            "navigate" | "openOverlay" | "swapOverlay" => Action::Board { kind, dest },
            "scrollTo" => Action::Layer {
                missing: !layers.contains(&dest),
                dest,
            },
            "changeTo" => Action::Plain(format!("{kind} {dest}")),
            "openUrl" => Action::Plain(format!("{kind} {}", text(action, "url"))),
            "setVariable" => Action::Plain(format!("{kind} {}", text(action, "variable"))),
            "setVariableMode" => Action::Plain(format!("{kind} {}", text(action, "collection"))),
            "conditional" => Action::Conditional(
                action
                    .get("blocks")
                    .and_then(Value::as_array)
                    .unwrap_or_default()
                    .iter()
                    .flat_map(|block| {
                        block
                            .get("actions")
                            .and_then(Value::as_array)
                            .unwrap_or_default()
                    })
                    .map(|inner| Self::action(inner, layers))
                    .collect(),
            ),
            _ => Action::Plain(kind),
        }
    }

    fn label(&self, action: &Action) -> String {
        match action {
            Action::Board { kind, dest } => match self.names.get(dest) {
                Some(name) => format!("{kind} {name} ({dest})"),
                None => format!("{kind} {dest} (missing)"),
            },
            Action::Layer { dest, missing } => {
                let note = if *missing { " (missing)" } else { "" };
                format!("scrollTo {dest}{note}")
            }
            Action::Plain(text) => text.clone(),
            Action::Conditional(inner) => {
                let labels: Vec<_> = inner.iter().map(|a| self.label(a)).collect();
                format!("conditional ({})", labels.join(", "))
            }
        }
    }

    fn is_missing(&self, action: &Action) -> bool {
        match action {
            Action::Board { dest, .. } => !self.names.contains_key(dest),
            Action::Layer { missing, .. } => *missing,
            Action::Plain(_) => false,
            Action::Conditional(inner) => inner.iter().any(|a| self.is_missing(a)),
        }
    }

    fn what(&self, interaction: &Interaction) -> String {
        let actions: Vec<_> = interaction.actions.iter().map(|a| self.label(a)).collect();
        format!("{} → {}", interaction.trigger, actions.join(", "))
    }

    /// The boards that no flow starts at and no interaction of another board
    /// of the page reaches.
    pub fn unreachable(&self) -> Vec<(&str, &str)> {
        let mut reached: Vec<&str> = self.flows.iter().map(|f| f.start.as_str()).collect();
        for interaction in &self.interactions {
            let from = &self.boards[interaction.board].id;
            let mut dests = Vec::new();
            reaches(&interaction.actions, &mut dests);
            reached.extend(dests.into_iter().filter(|dest| dest != from));
        }
        self.boards
            .iter()
            .filter(|board| !reached.contains(&board.id.as_str()))
            .map(|board| (board.name.as_str(), board.id.as_str()))
            .collect()
    }

    /// The lines of the interactions whose target is not in the doc.
    pub fn dangling(&self) -> Vec<String> {
        self.interactions
            .iter()
            .filter(|interaction| interaction.actions.iter().any(|a| self.is_missing(a)))
            .map(|interaction| {
                format!(
                    "{} ({}) {}",
                    interaction.path,
                    interaction.layer_id,
                    self.what(interaction)
                )
            })
            .collect()
    }

    pub fn render(&self) -> String {
        let mut out = format!("Page {} ({})\n", self.page_name, self.page_id);
        if self.flows.is_empty() {
            out.push_str("  flows: none\n");
        } else {
            let flows: Vec<String> = self
                .flows
                .iter()
                .map(|flow| match self.names.get(&flow.start) {
                    Some(name) => format!("{} → {name} ({})", flow.name, flow.start),
                    None => format!("{} → {} (missing)", flow.name, flow.start),
                })
                .collect();
            out.push_str(&format!("  flows: {}\n", flows.join("; ")));
        }
        for (index, board) in self.boards.iter().enumerate() {
            let own: Vec<_> = self
                .interactions
                .iter()
                .filter(|interaction| interaction.board == index)
                .collect();
            let count = match own.len() {
                0 => "no interactions".to_string(),
                1 => "1 interaction".to_string(),
                n => format!("{n} interactions"),
            };
            out.push_str(&format!("  {} ({}): {count}\n", board.name, board.id));
            for interaction in own {
                out.push_str(&format!(
                    "    {} ({}) {} · {}\n",
                    interaction.layer,
                    interaction.layer_id,
                    self.what(interaction),
                    interaction.style
                ));
            }
        }
        let unreachable = self.unreachable();
        if unreachable.is_empty() {
            out.push_str("  unreachable: none\n");
        } else {
            let list: Vec<String> = unreachable
                .iter()
                .map(|(name, id)| format!("{name} ({id})"))
                .collect();
            out.push_str(&format!("  unreachable: {}\n", list.join("; ")));
        }
        let dangling = self.dangling();
        if dangling.is_empty() {
            out.push_str("  dangling: none\n");
        }
        for line in dangling {
            out.push_str(&format!("  dangling: {line}\n"));
        }
        out
    }
}

/// The boards that actions point at, through conditional blocks too.
fn reaches<'a>(actions: &'a [Action], out: &mut Vec<&'a str>) {
    for action in actions {
        match action {
            Action::Board { dest, .. } => out.push(dest),
            Action::Conditional(inner) => reaches(inner, out),
            _ => {}
        }
    }
}

fn text(value: &Value, key: &str) -> String {
    value.str_at(key).unwrap_or_default().to_string()
}

type Found<'a> = (String, String, String, &'a [Value]);

/// Every layer of the board's subtree: the IDs of all of them, and those with
/// interactions as (name, ID, path from the board, interactions).
fn collect<'a>(node: &'a Value, parent: &str, ids: &mut Vec<String>, found: &mut Vec<Found<'a>>) {
    let name = text(node, "name");
    let path = if parent.is_empty() {
        name.clone()
    } else {
        format!("{parent}/{name}")
    };
    let id = text(node, "id");
    ids.push(id.clone());
    if let Some(interactions) = node.get("interactions").and_then(Value::as_array) {
        if !interactions.is_empty() {
            found.push((name, id, path.clone(), interactions));
        }
    }
    for child in node
        .get("children")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        collect(child, &path, ids, found);
    }
}

fn trigger_text(trigger: Option<&Value>) -> String {
    let Some(trigger) = trigger else {
        return "onClick".to_string();
    };
    if let Some(kind) = trigger.as_str() {
        return kind.to_string();
    }
    let kind = text(trigger, "type");
    match kind.as_str() {
        "afterDelay" => match trigger.get("ms") {
            Some(Value::Number(ms)) => format!("{kind} {ms} ms"),
            _ => kind,
        },
        "keyPress" => format!("{kind} {}", text(trigger, "key")),
        "" => "onClick".to_string(),
        _ => kind,
    }
}

/// `transition · duration · easing`, with the defaults of D26 filled in.
fn style_text(interaction: &Value) -> String {
    let transition = match interaction.get("transition") {
        None => "instant".to_string(),
        Some(transition) => {
            let kind = transition
                .as_str()
                .map_or_else(|| text(transition, "type"), str::to_string);
            match transition.str_at("direction") {
                Some(direction) => format!("{kind} {direction}"),
                None => kind,
            }
        }
    };
    let duration = match interaction.get("durationMs") {
        Some(Value::Number(ms)) => ms.as_str(),
        _ => DEFAULT_DURATION_MS,
    };
    let easing = match interaction.get("easing") {
        None => "easeOut".to_string(),
        Some(easing) => easing
            .as_str()
            .map_or_else(|| text(easing, "type"), str::to_string),
    };
    format!("{transition} · {duration} ms · {easing}")
}
