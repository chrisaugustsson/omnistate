//! Penpot prototype interactions, mapped to OmniState's interactions.
//!
//! Penpot stores an interaction on the shape that fires it: an event type, an
//! action type, the destination shape and an optional animation. The mapping
//! is FigCraft's `crates/format/src/penpot/prototype.rs` (MIT OR Apache-2.0;
//! see `NOTICE`). A destination is a Penpot shape id, which becomes a node ID
//! only once every page is converted, so a shape's interactions are read first
//! and resolved after the last page.

use std::collections::HashMap;

use omni_core::design::NodeId;
use omni_core::design::proto::{
    Action, Direction, Easing, Interaction, OverlayPosition, Transition, Trigger,
};
use serde_json::Value;

use super::ReportEntry;

/// A Penpot shape id.
pub(super) type PenpotId = String;
/// The type of a node, as the design writes it: `board`, `frame`, `text`...
pub(super) type Kind = String;

/// Longest animation Penpot may declare, in milliseconds.
const MAX_DURATION_MS: u32 = 600_000;
/// Longest "after delay" wait, in milliseconds.
const MAX_DELAY_MS: u32 = 600_000;
/// Penpot's own default wait for a delayed trigger.
const DEFAULT_DELAY_MS: u32 = 600;
/// Most interactions read from one shape.
pub(super) const MAX_INTERACTIONS: usize = 64;
/// Most flows read from one page.
pub(super) const MAX_FLOWS: usize = 8;

/// Where an interaction was found, for the report.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Source {
    pub page: String,
    pub board: String,
    pub board_id: String,
    pub shape_id: String,
    pub name: String,
}

/// One Penpot interaction, read from a shape. The destination of its action is
/// empty until the ids are resolved.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PendingInteraction {
    pub interaction: Interaction,
    /// The Penpot shape id the action points at, if it points at one.
    pub destination: Option<PenpotId>,
    pub source: Source,
}

/// Penpot's `event-type` names, mapped to triggers.
fn trigger(event: &str, delay: Option<u64>) -> Option<Trigger> {
    Some(match event {
        "click" => Trigger::OnClick,
        "mouse-press" => Trigger::OnPress,
        "mouse-over" => Trigger::OnHover,
        "mouse-enter" => Trigger::MouseEnter,
        "mouse-leave" => Trigger::MouseLeave,
        "after-delay" => Trigger::AfterDelay {
            ms: delay.map_or(DEFAULT_DELAY_MS, |ms| {
                ms.min(u64::from(MAX_DELAY_MS)) as u32
            }),
        },
        _ => return None,
    })
}

fn overlay_position(position: &str) -> Option<OverlayPosition> {
    Some(match position {
        "manual" => OverlayPosition::Manual,
        "center" => OverlayPosition::Center,
        "top-left" => OverlayPosition::TopLeft,
        "top-right" => OverlayPosition::TopRight,
        "top-center" => OverlayPosition::TopCenter,
        "bottom-left" => OverlayPosition::BottomLeft,
        "bottom-right" => OverlayPosition::BottomRight,
        "bottom-center" => OverlayPosition::BottomCenter,
        _ => return None,
    })
}

/// The transition, its duration and its easing. Without an animation the
/// change is instant.
fn transition(animation: Option<&Value>, overlay: bool) -> (Transition, u32, Easing) {
    let Some(animation) = animation else {
        return (Transition::Instant, 0, Easing::Linear);
    };
    let duration = animation
        .get("duration")
        .and_then(Value::as_f64)
        .filter(|duration| duration.is_finite() && *duration >= 0.0)
        .map_or(0, |duration| {
            duration.min(f64::from(MAX_DURATION_MS)) as u32
        });
    let easing = match animation.get("easing").and_then(Value::as_str) {
        Some("ease" | "ease-in-out") => Easing::EaseInOut,
        Some("ease-in") => Easing::EaseIn,
        Some("ease-out") => Easing::EaseOut,
        _ => Easing::Linear,
    };
    // Penpot's direction names the way the screen moves, as ours does.
    let direction = match animation.get("direction").and_then(Value::as_str) {
        Some("right") => Direction::Right,
        Some("up") => Direction::Top,
        Some("down") => Direction::Bottom,
        _ => Direction::Left,
    };
    // An overlay's animation is the overlay coming in: its "way" does not
    // apply, and a push is not an overlay animation.
    let way = if overlay {
        Some("in")
    } else {
        animation.get("way").and_then(Value::as_str)
    };
    let transition = match (animation.get("animationType").and_then(Value::as_str), way) {
        (Some("dissolve"), _) => Transition::Dissolve,
        (Some("slide"), Some("out")) => Transition::SlideOut { direction },
        (Some("slide"), _) => Transition::SlideIn { direction },
        (Some("push"), _) if !overlay => Transition::Push { direction },
        _ => Transition::Instant,
    };
    (transition, duration, easing)
}

fn open_overlay(item: &Value, position: OverlayPosition) -> Action {
    // A manual overlay sits at an offset from the layer that opened it.
    let offset = match (position, item.get("overlayPosition")) {
        (OverlayPosition::Manual, Some(at)) => {
            let x = at.get("x").and_then(Value::as_f64).unwrap_or(0.0);
            let y = at.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            (x.is_finite() && y.is_finite()).then_some([
                x.clamp(-100_000.0, 100_000.0),
                y.clamp(-100_000.0, 100_000.0),
            ])
        }
        _ => None,
    };
    let flag = |key: &str| item.get(key).and_then(Value::as_bool).unwrap_or(false);
    Action::OpenOverlay {
        dest: String::new(),
        position,
        // Penpot leaves "close when clicking outside" off unless it is asked for.
        close_on_outside: flag("closeClickOutside"),
        // Penpot draws a translucent black sheet behind an overlay that asks for one.
        background: flag("backgroundOverlay").then(|| "#00000033".to_string()),
        offset,
        anchor: Default::default(),
    }
}

/// Maps the `interactions` of one shape. Each one that is dropped or only
/// approximated, and a list cut at the cap, adds a reason to `warnings`.
pub(super) fn shape_interactions(
    items: &[Value],
    warnings: &mut Vec<String>,
) -> Vec<PendingInteraction> {
    if items.len() > MAX_INTERACTIONS {
        warnings.push(format!(
            "the shape has {} prototype interactions; the first {MAX_INTERACTIONS} are kept and the other {} are left out",
            items.len(),
            items.len() - MAX_INTERACTIONS
        ));
    }
    let mut out = Vec::new();
    for item in items.iter().take(MAX_INTERACTIONS) {
        let text = |key: &str| item.get(key).and_then(Value::as_str);
        let event = text("eventType");
        let delay = item.get("delay").and_then(Value::as_u64);
        let Some(trigger) = event.and_then(|event| trigger(event, delay)) else {
            warnings.push(format!(
                "the prototype interaction with event type '{}' is not converted and is left out",
                event.unwrap_or("unknown")
            ));
            continue;
        };
        let kind = text("actionType");
        let overlay = matches!(kind, Some("open-overlay" | "toggle-overlay"));
        let mut destination = text("destination").map(str::to_owned);
        let action = match kind {
            Some("navigate") => Action::Navigate {
                dest: String::new(),
                preserve_scroll: item
                    .get("preserveScroll")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
            Some("open-overlay" | "toggle-overlay") => {
                let position = match text("overlayPosType") {
                    None => Some(OverlayPosition::Center),
                    Some(position) => overlay_position(position),
                };
                let Some(position) = position else {
                    warnings.push(format!(
                        "the prototype interaction with overlay position '{}' is not converted and is left out",
                        text("overlayPosType").unwrap_or("unknown")
                    ));
                    continue;
                };
                if kind == Some("toggle-overlay") {
                    warnings.push("toggle-overlay imported as open overlay".to_string());
                }
                open_overlay(item, position)
            }
            Some("close-overlay") => Action::CloseOverlay,
            Some("prev-screen") => Action::Back,
            Some("open-url") => match text("url").map(str::trim).filter(|url| !url.is_empty()) {
                Some(url) => Action::OpenUrl {
                    url: url.chars().take(2048).collect(),
                    new_tab: true,
                },
                None => {
                    warnings.push(
                        "the prototype interaction with action type 'open-url' names no url and is left out"
                            .to_string(),
                    );
                    continue;
                }
            },
            other => {
                warnings.push(format!(
                    "the prototype interaction with action type '{}' is not converted and is left out",
                    other.unwrap_or("unknown")
                ));
                continue;
            }
        };
        if action.dest_text().is_some() {
            if destination.is_none() {
                warnings.push(format!(
                    "the prototype interaction with action type '{}' names no destination and is left out",
                    kind.unwrap_or("unknown")
                ));
                continue;
            }
        } else {
            destination = None;
        }
        let (transition, duration_ms, easing) = transition(item.get("animation"), overlay);
        out.push(PendingInteraction {
            interaction: Interaction {
                trigger,
                actions: vec![action],
                transition,
                duration_ms,
                easing,
                delay_ms: 0,
            },
            destination,
            source: Source::default(),
        });
    }
    out
}

/// The same action with another destination.
fn with_dest(action: Action, id: NodeId) -> Action {
    let dest = id.to_string();
    match action {
        Action::Navigate {
            preserve_scroll, ..
        } => Action::Navigate {
            dest,
            preserve_scroll,
        },
        Action::OpenOverlay {
            position,
            close_on_outside,
            background,
            offset,
            anchor,
            ..
        } => Action::OpenOverlay {
            dest,
            position,
            close_on_outside,
            background,
            offset,
            anchor,
        },
        other => other,
    }
}

/// Gives each pending interaction the node its destination became. A
/// destination that is not a top-level board of the file (a left-out shape, an
/// unknown id, a nested frame) drops the interaction and adds a report entry.
pub(super) fn resolve_destinations(
    pending: Vec<PendingInteraction>,
    map: &HashMap<PenpotId, (NodeId, Kind)>,
) -> (Vec<Interaction>, Vec<ReportEntry>) {
    let mut interactions = Vec::new();
    let mut entries = Vec::new();
    for PendingInteraction {
        mut interaction,
        destination,
        source,
    } in pending
    {
        if let Some(destination) = destination {
            let Some((id, _)) = map.get(&destination).filter(|(_, kind)| kind == "board") else {
                entries.push(ReportEntry {
                    board: source.board,
                    board_id: source.board_id,
                    shape_id: source.shape_id,
                    name: source.name,
                    property: Some("interactions".to_string()),
                    reason: format!("prototype link to {destination} was not imported"),
                });
                continue;
            };
            interaction.actions = interaction
                .actions
                .into_iter()
                .map(|action| with_dest(action, *id))
                .collect();
        }
        interactions.push(interaction);
    }
    (interactions, entries)
}
