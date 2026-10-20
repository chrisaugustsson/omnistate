//! Design nodes and their JSON form.

use super::component::{Bind, Overrides, PropDefs, PropValues};
use super::id::NodeId;
use super::json::{Number, Object, entry, first_set, some, some_number, some_object, some_objects};
use super::layout::{GridPlacement, Layout, LayoutMode, Margin, SelfAlignment, Sizing};
use super::path::PathData;
use super::style::{Fill, FontStyle, ImageFit, Radius, Shadow, Stroke, TextAlign, TextDecoration};
use super::value::{AttachmentRef, Color, FontFamily, Length, TokenOr, some_non_negative};
use serde::de::Error as _;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::HashSet;
use std::ops::RangeInclusive;

keywords! {
    /// The type of a design node.
    pub enum NodeKind {
        /// A page of a design doc. Its children are boards.
        Page = "page",
        /// A top-level frame on a page.
        Board = "board",
        /// A box that lays out its children.
        Frame = "frame",
        /// Plain text in one style.
        Text = "text",
        /// An image from an attachment.
        Image = "image",
        /// A shape drawn from SVG path data.
        Vector = "vector",
        /// A placed component, defined in the design system doc.
        Instance = "instance",
        /// A component: a tree of layers with properties, in the design system
        /// doc.
        Component = "component",
    }
}

/// A design node and its subtree.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    id: NodeId,
    name: String,
    visible: bool,
    placement: Placement,
    /// The property of its component that the layer takes.
    bind: Option<Bind>,
    data: NodeData,
}

impl Node {
    pub fn id(&self) -> &NodeId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> NodeKind {
        self.data.kind()
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    /// The children of a page, board, frame or component, in order. Other
    /// nodes have none.
    pub fn children(&self) -> &[Node] {
        match &self.data {
            NodeData::Page(page) => &page.children,
            NodeData::Board(frame) | NodeData::Frame(frame) => &frame.children,
            NodeData::Component(component) => &component.frame.children,
            NodeData::Text(_)
            | NodeData::Image(_)
            | NodeData::Vector(_)
            | NodeData::Instance(_) => &[],
        }
    }

    /// How the node is placed in its parent. Pages are not placed.
    pub fn placement(&self) -> &Placement {
        &self.placement
    }

    /// The node's type, with the properties of that type.
    pub fn data(&self) -> &NodeData {
        &self.data
    }

    /// The property of its component that a layer of a component takes.
    pub fn bind(&self) -> Option<&Bind> {
        self.bind.as_ref()
    }
}

/// A node's type, with the properties of that type.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeData {
    Page(Page),
    /// A board has every property of a frame.
    Board(Frame),
    Frame(Frame),
    Text(Text),
    Image(Image),
    Vector(Vector),
    Instance(Instance),
    Component(Component),
}

impl NodeData {
    pub fn kind(&self) -> NodeKind {
        match self {
            NodeData::Page(_) => NodeKind::Page,
            NodeData::Board(_) => NodeKind::Board,
            NodeData::Frame(_) => NodeKind::Frame,
            NodeData::Text(_) => NodeKind::Text,
            NodeData::Image(_) => NodeKind::Image,
            NodeData::Vector(_) => NodeKind::Vector,
            NodeData::Instance(_) => NodeKind::Instance,
            NodeData::Component(_) => NodeKind::Component,
        }
    }
}

/// A page: a canvas whose children are boards.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub children: Vec<Node>,
}

/// A frame or a board: a box that lays out its children.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub size: Size,
    pub layout: Layout,
    pub fills: Option<Vec<Fill>>,
    pub strokes: Option<Vec<Stroke>>,
    pub radius: Option<Radius>,
    pub shadows: Option<Vec<TokenOr<Shadow>>>,
    /// From 0, transparent, to 1, opaque.
    pub opacity: Option<TokenOr<f64>>,
    /// Whether children are clipped to the box.
    pub clip: Option<bool>,
    pub children: Vec<Node>,
}

/// Plain text in one style.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub size: Size,
    /// The text, kept exactly as written. It is never markup.
    pub content: String,
    pub font_family: TokenOr<FontFamily>,
    pub font_size: Length,
    /// From 1 to 1000, as CSS `font-weight`.
    pub font_weight: Option<TokenOr<f64>>,
    pub font_style: Option<FontStyle>,
    /// A multiple of the font size, as a unitless CSS `line-height`.
    pub line_height: Option<TokenOr<f64>>,
    pub letter_spacing: Option<Length>,
    pub text_align: Option<TextAlign>,
    pub text_decoration: Option<TextDecoration>,
    pub color: Option<TokenOr<Color>>,
    pub opacity: Option<TokenOr<f64>>,
}

/// An image from an attachment.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub size: Size,
    pub image: AttachmentRef,
    pub fit: ImageFit,
    pub strokes: Option<Vec<Stroke>>,
    pub radius: Option<Radius>,
    pub shadows: Option<Vec<TokenOr<Shadow>>>,
    pub opacity: Option<TokenOr<f64>>,
}

/// A shape drawn from SVG path data.
#[derive(Clone, Debug, PartialEq)]
pub struct Vector {
    pub size: Size,
    pub path: PathData,
    pub fills: Option<Vec<Fill>>,
    pub strokes: Option<Vec<Stroke>>,
    pub shadows: Option<Vec<TokenOr<Shadow>>>,
    pub opacity: Option<TokenOr<f64>>,
}

/// A component: a frame whose layers may take properties, defined at the top
/// of the design system doc.
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    /// The component's box and its tree of layers.
    pub frame: Frame,
    /// The properties that instances give values to.
    pub props: Option<PropDefs>,
}

/// A placed component. It takes its component's size on each axis it leaves
/// out, and stores nothing of the component's tree or style: only the values
/// of its properties and its overrides.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    /// The node ID of the component in the design system doc.
    pub component: NodeId,
    pub size: Size<Option<Sizing>>,
    pub props: Option<PropValues>,
    /// What the instance changes on nodes of the component, by node ID.
    pub overrides: Option<Overrides>,
}

/// How a node is placed in its parent. Layout computes the position of every
/// child that flows in a flex or grid layout, so only the boards of a page,
/// the children of a `none` layout and children taken out of the flow with
/// `absolute` have an `x` and a `y`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    /// Takes the node out of its flex or grid parent's flow.
    pub absolute: Option<bool>,
    /// The left edge, in pixels from the parent's left edge.
    pub x: Option<f64>,
    /// The top edge, in pixels from the parent's top edge.
    pub y: Option<f64>,
    /// The columns of a grid layout the node is placed in.
    pub grid_column: Option<GridPlacement>,
    /// The rows of a grid layout the node is placed in.
    pub grid_row: Option<GridPlacement>,
    /// The space outside each edge of the node, as CSS `margin`.
    pub margin: Option<Margin>,
    /// How a child of a grid aligns itself across its grid area, as CSS
    /// `justify-self`.
    pub justify_self: Option<SelfAlignment>,
    /// How a child of a grid aligns itself down its grid area, as CSS
    /// `align-self`.
    pub align_self: Option<SelfAlignment>,
}

impl Placement {
    /// Whether the node is taken out of its flex or grid parent's flow.
    pub fn is_absolute(&self) -> bool {
        self.absolute == Some(true)
    }
}

/// How a node sizes each axis, and the bounds of its size.
#[derive(Clone, Debug, PartialEq)]
pub struct Size<S = Sizing> {
    pub width: S,
    pub height: S,
    pub min_width: Option<Length>,
    pub max_width: Option<Length>,
    pub min_height: Option<Length>,
    pub max_height: Option<Length>,
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &self.id)?;
        map.serialize_entry("type", &self.kind())?;
        map.serialize_entry("name", &self.name)?;
        map.serialize_entry("visible", &self.visible)?;
        let placement = &self.placement;
        entry(&mut map, "absolute", placement.absolute.as_ref())?;
        entry(&mut map, "x", placement.x.map(Number).as_ref())?;
        entry(&mut map, "y", placement.y.map(Number).as_ref())?;
        entry(&mut map, "gridColumn", placement.grid_column.as_ref())?;
        entry(&mut map, "gridRow", placement.grid_row.as_ref())?;
        entry(&mut map, "margin", placement.margin.as_ref())?;
        entry(&mut map, "justifySelf", placement.justify_self.as_ref())?;
        entry(&mut map, "alignSelf", placement.align_self.as_ref())?;
        entry(&mut map, "bind", self.bind.as_ref())?;
        match &self.data {
            NodeData::Page(page) => map.serialize_entry("children", &page.children)?,
            NodeData::Board(frame) | NodeData::Frame(frame) => {
                write_size(&mut map, &frame.size)?;
                map.serialize_entry("layout", &frame.layout)?;
                entry(&mut map, "fills", frame.fills.as_ref())?;
                entry(&mut map, "strokes", frame.strokes.as_ref())?;
                entry(&mut map, "radius", frame.radius.as_ref())?;
                entry(&mut map, "shadows", frame.shadows.as_ref())?;
                entry(&mut map, "opacity", frame.opacity.as_ref())?;
                entry(&mut map, "clip", frame.clip.as_ref())?;
                map.serialize_entry("children", &frame.children)?;
            }
            NodeData::Text(text) => {
                write_size(&mut map, &text.size)?;
                map.serialize_entry("content", &text.content)?;
                map.serialize_entry("fontFamily", &text.font_family)?;
                map.serialize_entry("fontSize", &text.font_size)?;
                entry(&mut map, "fontWeight", text.font_weight.as_ref())?;
                entry(&mut map, "fontStyle", text.font_style.as_ref())?;
                entry(&mut map, "lineHeight", text.line_height.as_ref())?;
                entry(&mut map, "letterSpacing", text.letter_spacing.as_ref())?;
                entry(&mut map, "textAlign", text.text_align.as_ref())?;
                entry(&mut map, "textDecoration", text.text_decoration.as_ref())?;
                entry(&mut map, "color", text.color.as_ref())?;
                entry(&mut map, "opacity", text.opacity.as_ref())?;
            }
            NodeData::Image(image) => {
                write_size(&mut map, &image.size)?;
                map.serialize_entry("image", &image.image)?;
                map.serialize_entry("fit", &image.fit)?;
                entry(&mut map, "strokes", image.strokes.as_ref())?;
                entry(&mut map, "radius", image.radius.as_ref())?;
                entry(&mut map, "shadows", image.shadows.as_ref())?;
                entry(&mut map, "opacity", image.opacity.as_ref())?;
            }
            NodeData::Vector(vector) => {
                write_size(&mut map, &vector.size)?;
                map.serialize_entry("path", &vector.path)?;
                entry(&mut map, "fills", vector.fills.as_ref())?;
                entry(&mut map, "strokes", vector.strokes.as_ref())?;
                entry(&mut map, "shadows", vector.shadows.as_ref())?;
                entry(&mut map, "opacity", vector.opacity.as_ref())?;
            }
            NodeData::Instance(instance) => {
                write_size(&mut map, &instance.size)?;
                map.serialize_entry("component", &instance.component)?;
                entry(&mut map, "props", instance.props.as_ref())?;
                entry(&mut map, "overrides", instance.overrides.as_ref())?;
            }
            NodeData::Component(component) => {
                let frame = &component.frame;
                write_size(&mut map, &frame.size)?;
                map.serialize_entry("layout", &frame.layout)?;
                entry(&mut map, "fills", frame.fills.as_ref())?;
                entry(&mut map, "strokes", frame.strokes.as_ref())?;
                entry(&mut map, "radius", frame.radius.as_ref())?;
                entry(&mut map, "shadows", frame.shadows.as_ref())?;
                entry(&mut map, "opacity", frame.opacity.as_ref())?;
                entry(&mut map, "clip", frame.clip.as_ref())?;
                entry(&mut map, "props", component.props.as_ref())?;
                map.serialize_entry("children", &frame.children)?;
            }
        }
        map.end()
    }
}

/// The sizing of one axis, which instances may leave out.
trait Axis {
    fn sizing(&self) -> Option<&Sizing>;
}

impl Axis for Sizing {
    fn sizing(&self) -> Option<&Sizing> {
        Some(self)
    }
}

impl Axis for Option<Sizing> {
    fn sizing(&self) -> Option<&Sizing> {
        self.as_ref()
    }
}

fn write_size<M: SerializeMap, A: Axis>(map: &mut M, size: &Size<A>) -> Result<(), M::Error> {
    entry(map, "width", size.width.sizing())?;
    entry(map, "height", size.height.sizing())?;
    entry(map, "minWidth", size.min_width.as_ref())?;
    entry(map, "maxWidth", size.max_width.as_ref())?;
    entry(map, "minHeight", size.min_height.as_ref())?;
    entry(map, "maxHeight", size.max_height.as_ref())
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Subtree(node) = Subtree::deserialize(deserializer)?;
        check_unique_ids(&node).map_err(D::Error::custom)?;
        check_binds_inside_components(&node).map_err(D::Error::custom)?;
        Ok(node)
    }
}

/// A node read with its subtree. Unlike [`Node`], it leaves checking that IDs
/// are unique to the root of the tree.
struct Subtree(Node);

impl<'de> Deserialize<'de> for Subtree {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Object(raw) = Object::<RawNode>::deserialize(deserializer)?;
        raw.into_node().map(Subtree).map_err(D::Error::custom)
    }
}

/// Every node property, before the node's type says which ones apply.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawNode {
    id: NodeId,
    #[serde(rename = "type")]
    kind: NodeKind,
    name: String,
    #[serde(default, deserialize_with = "some")]
    visible: Option<bool>,
    #[serde(default, deserialize_with = "some")]
    absolute: Option<bool>,
    #[serde(default, deserialize_with = "some_number")]
    x: Option<f64>,
    #[serde(default, deserialize_with = "some_number")]
    y: Option<f64>,
    #[serde(default, deserialize_with = "some_object")]
    grid_column: Option<GridPlacement>,
    #[serde(default, deserialize_with = "some_object")]
    grid_row: Option<GridPlacement>,
    #[serde(default, deserialize_with = "some_object")]
    margin: Option<Margin>,
    #[serde(default, deserialize_with = "some")]
    justify_self: Option<SelfAlignment>,
    #[serde(default, deserialize_with = "some")]
    align_self: Option<SelfAlignment>,
    #[serde(default, deserialize_with = "some")]
    width: Option<Sizing>,
    #[serde(default, deserialize_with = "some")]
    height: Option<Sizing>,
    #[serde(default, deserialize_with = "some_non_negative")]
    min_width: Option<Length>,
    #[serde(default, deserialize_with = "some_non_negative")]
    max_width: Option<Length>,
    #[serde(default, deserialize_with = "some_non_negative")]
    min_height: Option<Length>,
    #[serde(default, deserialize_with = "some_non_negative")]
    max_height: Option<Length>,
    #[serde(default, deserialize_with = "some")]
    layout: Option<Layout>,
    #[serde(default, deserialize_with = "some")]
    fills: Option<Vec<Fill>>,
    #[serde(default, deserialize_with = "some_objects")]
    strokes: Option<Vec<Stroke>>,
    #[serde(default, deserialize_with = "some_object")]
    radius: Option<Radius>,
    #[serde(default, deserialize_with = "some")]
    shadows: Option<Vec<TokenOr<Shadow>>>,
    #[serde(default, deserialize_with = "some")]
    opacity: Option<TokenOr<f64>>,
    #[serde(default, deserialize_with = "some")]
    clip: Option<bool>,
    #[serde(default, deserialize_with = "some")]
    content: Option<String>,
    #[serde(default, deserialize_with = "some")]
    font_family: Option<TokenOr<FontFamily>>,
    #[serde(default, deserialize_with = "some_non_negative")]
    font_size: Option<Length>,
    #[serde(default, deserialize_with = "some")]
    font_weight: Option<TokenOr<f64>>,
    #[serde(default, deserialize_with = "some")]
    font_style: Option<FontStyle>,
    #[serde(default, deserialize_with = "some_non_negative")]
    line_height: Option<TokenOr<f64>>,
    #[serde(default, deserialize_with = "some")]
    letter_spacing: Option<Length>,
    #[serde(default, deserialize_with = "some")]
    text_align: Option<TextAlign>,
    #[serde(default, deserialize_with = "some")]
    text_decoration: Option<TextDecoration>,
    #[serde(default, deserialize_with = "some")]
    color: Option<TokenOr<Color>>,
    #[serde(default, deserialize_with = "some")]
    image: Option<AttachmentRef>,
    #[serde(default, deserialize_with = "some")]
    fit: Option<ImageFit>,
    #[serde(default, deserialize_with = "some")]
    path: Option<PathData>,
    #[serde(default, deserialize_with = "some")]
    component: Option<NodeId>,
    #[serde(default, deserialize_with = "some")]
    props: Option<Value>,
    #[serde(default, deserialize_with = "some")]
    overrides: Option<Value>,
    #[serde(default, deserialize_with = "some")]
    bind: Option<Value>,
    #[serde(default, deserialize_with = "some")]
    children: Option<Vec<Subtree>>,
}

impl RawNode {
    /// Takes the properties of the node's type, and rejects any other.
    fn into_node(mut self) -> Result<Node, String> {
        let visible = self.visible.take().unwrap_or(true);
        let placement = match self.kind {
            NodeKind::Page | NodeKind::Component => Placement::default(),
            _ => self.placement(),
        };
        let bind = match self.kind {
            NodeKind::Frame
            | NodeKind::Text
            | NodeKind::Image
            | NodeKind::Vector
            | NodeKind::Instance => self
                .bind
                .take()
                .map(|bind| Bind::from_value(&bind))
                .transpose()?,
            _ => None,
        };
        let data = match self.kind {
            NodeKind::Page => NodeData::Page(self.page()?),
            NodeKind::Board => NodeData::Board(self.frame()?),
            NodeKind::Frame => NodeData::Frame(self.frame()?),
            NodeKind::Text => NodeData::Text(self.text()?),
            NodeKind::Image => NodeData::Image(self.image()?),
            NodeKind::Vector => NodeData::Vector(self.vector()?),
            NodeKind::Instance => NodeData::Instance(self.instance()?),
            NodeKind::Component => NodeData::Component(self.component()?),
        };
        if let Some(key) = self.unused() {
            return Err(format!("{} nodes have no `{key}`", self.kind));
        }
        Ok(Node {
            id: self.id,
            name: self.name,
            visible,
            placement,
            bind,
            data,
        })
    }

    fn placement(&mut self) -> Placement {
        Placement {
            absolute: self.absolute.take(),
            x: self.x.take(),
            y: self.y.take(),
            grid_column: self.grid_column.take(),
            grid_row: self.grid_row.take(),
            margin: self.margin.take(),
            justify_self: self.justify_self.take(),
            align_self: self.align_self.take(),
        }
    }

    fn page(&mut self) -> Result<Page, String> {
        let children = self.children();
        check_children(self.id, self.kind, None, &children)?;
        Ok(Page { children })
    }

    fn frame(&mut self) -> Result<Frame, String> {
        let size = self.size()?;
        let layout = required(self.kind, "layout", self.layout.take())?;
        let children = self.children();
        check_children(self.id, self.kind, Some(&layout), &children)?;
        Ok(Frame {
            size,
            layout,
            fills: self.fills.take(),
            strokes: self.strokes.take(),
            radius: self.radius.take(),
            shadows: self.shadows.take(),
            opacity: self.opacity()?,
            clip: self.clip.take(),
            children,
        })
    }

    fn text(&mut self) -> Result<Text, String> {
        let kind = self.kind;
        let font_weight = self.font_weight.take();
        check_range(
            "fontWeight",
            font_weight.as_ref(),
            1.0..=1000.0,
            "font weights go from 1 to 1000",
        )?;
        Ok(Text {
            size: self.size()?,
            content: required(kind, "content", self.content.take())?,
            font_family: required(kind, "fontFamily", self.font_family.take())?,
            font_size: required(kind, "fontSize", self.font_size.take())?,
            font_weight,
            font_style: self.font_style.take(),
            line_height: self.line_height.take(),
            letter_spacing: self.letter_spacing.take(),
            text_align: self.text_align.take(),
            text_decoration: self.text_decoration.take(),
            color: self.color.take(),
            opacity: self.opacity()?,
        })
    }

    fn image(&mut self) -> Result<Image, String> {
        let kind = self.kind;
        check_stroke_align(kind, self.strokes.as_deref())?;
        Ok(Image {
            size: self.size()?,
            image: required(kind, "image", self.image.take())?,
            fit: required(kind, "fit", self.fit.take())?,
            strokes: self.strokes.take(),
            radius: self.radius.take(),
            shadows: self.shadows.take(),
            opacity: self.opacity()?,
        })
    }

    fn vector(&mut self) -> Result<Vector, String> {
        check_stroke_align(self.kind, self.strokes.as_deref())?;
        Ok(Vector {
            size: self.size()?,
            path: required(self.kind, "path", self.path.take())?,
            fills: self.fills.take(),
            strokes: self.strokes.take(),
            shadows: self.shadows.take(),
            opacity: self.opacity()?,
        })
    }

    fn instance(&mut self) -> Result<Instance, String> {
        let width = self.width.take();
        let height = self.height.take();
        Ok(Instance {
            component: required(self.kind, "component", self.component.take())?,
            size: self.bounds(width, height),
            props: self
                .props
                .take()
                .map(|props| PropValues::from_value(&props))
                .transpose()?,
            overrides: self
                .overrides
                .take()
                .map(|overrides| Overrides::from_value(&overrides))
                .transpose()?,
        })
    }

    fn component(&mut self) -> Result<Component, String> {
        let props = self
            .props
            .take()
            .map(|props| PropDefs::from_value(&props))
            .transpose()?;
        let frame = self.frame()?;
        check_binds(
            props.as_ref().unwrap_or(&PropDefs::default()),
            &frame.children,
        )?;
        Ok(Component { frame, props })
    }

    fn size(&mut self) -> Result<Size, String> {
        let width = required(self.kind, "width", self.width.take())?;
        let height = required(self.kind, "height", self.height.take())?;
        Ok(self.bounds(width, height))
    }

    fn bounds<S>(&mut self, width: S, height: S) -> Size<S> {
        Size {
            width,
            height,
            min_width: self.min_width.take(),
            max_width: self.max_width.take(),
            min_height: self.min_height.take(),
            max_height: self.max_height.take(),
        }
    }

    fn opacity(&mut self) -> Result<Option<TokenOr<f64>>, String> {
        let opacity = self.opacity.take();
        check_range(
            "opacity",
            opacity.as_ref(),
            0.0..=1.0,
            "opacity goes from 0 to 1",
        )?;
        Ok(opacity)
    }

    fn children(&mut self) -> Vec<Node> {
        let children = self.children.take().unwrap_or_default();
        children.into_iter().map(|Subtree(node)| node).collect()
    }

    /// The first property left that the node's type has no use for.
    fn unused(&self) -> Option<&'static str> {
        first_set([
            ("absolute", self.absolute.is_some()),
            ("x", self.x.is_some()),
            ("y", self.y.is_some()),
            ("gridColumn", self.grid_column.is_some()),
            ("gridRow", self.grid_row.is_some()),
            ("margin", self.margin.is_some()),
            ("justifySelf", self.justify_self.is_some()),
            ("alignSelf", self.align_self.is_some()),
            ("width", self.width.is_some()),
            ("height", self.height.is_some()),
            ("minWidth", self.min_width.is_some()),
            ("maxWidth", self.max_width.is_some()),
            ("minHeight", self.min_height.is_some()),
            ("maxHeight", self.max_height.is_some()),
            ("layout", self.layout.is_some()),
            ("fills", self.fills.is_some()),
            ("strokes", self.strokes.is_some()),
            ("radius", self.radius.is_some()),
            ("shadows", self.shadows.is_some()),
            ("opacity", self.opacity.is_some()),
            ("clip", self.clip.is_some()),
            ("content", self.content.is_some()),
            ("fontFamily", self.font_family.is_some()),
            ("fontSize", self.font_size.is_some()),
            ("fontWeight", self.font_weight.is_some()),
            ("fontStyle", self.font_style.is_some()),
            ("lineHeight", self.line_height.is_some()),
            ("letterSpacing", self.letter_spacing.is_some()),
            ("textAlign", self.text_align.is_some()),
            ("textDecoration", self.text_decoration.is_some()),
            ("color", self.color.is_some()),
            ("image", self.image.is_some()),
            ("fit", self.fit.is_some()),
            ("path", self.path.is_some()),
            ("component", self.component.is_some()),
            ("props", self.props.is_some()),
            ("overrides", self.overrides.is_some()),
            ("bind", self.bind.is_some()),
            ("children", self.children.is_some()),
        ])
    }
}

fn required<T>(kind: NodeKind, key: &str, value: Option<T>) -> Result<T, String> {
    value.ok_or_else(|| format!("{kind} nodes need `{key}`"))
}

/// Checks that the strokes of `kind` leave out `align`, which only the strokes
/// of frames and boards set: a stroke of an image lies inside its edge, and
/// the stroke of a vector is centered on its path.
fn check_stroke_align(kind: NodeKind, strokes: Option<&[Stroke]>) -> Result<(), String> {
    match strokes {
        Some(strokes) if strokes.iter().any(|stroke| stroke.align.is_some()) => Err(format!(
            "strokes of {kind} nodes have no `align`: only the strokes of frames and boards sit inside, centered on or outside their edge"
        )),
        _ => Ok(()),
    }
}

/// Checks that a literal number lies in `range`. A token's value is checked
/// where the token is defined.
fn check_range(
    key: &str,
    value: Option<&TokenOr<f64>>,
    range: RangeInclusive<f64>,
    rule: &str,
) -> Result<(), String> {
    match value {
        Some(TokenOr::Literal(number)) if !range.contains(number) => {
            Err(format!("`{key}` is {number}, but {rule}"))
        }
        _ => Ok(()),
    }
}

/// Checks the `bind` of every layer under a component against the component's
/// properties.
fn check_binds(props: &PropDefs, layers: &[Node]) -> Result<(), String> {
    for layer in layers {
        if let Some(bind) = layer.bind()
            && let Some(problem) = bind.problem(layer.kind(), props)
        {
            return Err(format!("the bind of `{}`: {problem}", layer.id()));
        }
        check_binds(props, layer.children())?;
    }
    Ok(())
}

/// Checks that only the layers of a component take properties with `bind`.
/// The tree of a component is checked with the component.
fn check_binds_inside_components(root: &Node) -> Result<(), String> {
    if root.kind() == NodeKind::Component {
        return Ok(());
    }
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if node.bind().is_some() {
            return Err(format!(
                "`bind` is for the layers of a component, but `{}` sets it outside one",
                node.id()
            ));
        }
        pending.extend(node.children());
    }
    Ok(())
}

/// Checks where `children` sit: pages hold only boards, boards sit only on
/// pages, layout computes the position of children that flow in a flex or grid
/// layout, and only the children of grids are placed on grid lines and align
/// themselves in their grid areas.
fn check_children(
    parent: NodeId,
    kind: NodeKind,
    layout: Option<&Layout>,
    children: &[Node],
) -> Result<(), String> {
    let mode = layout.map(Layout::mode);
    for child in children {
        let id = child.id();
        match (kind, child.kind()) {
            (NodeKind::Page, NodeKind::Board) => {}
            (_, NodeKind::Component) => {
                return Err(format!(
                    "components sit only at the top of the design system doc, but the component `{id}` is inside the {kind} `{parent}`"
                ));
            }
            (NodeKind::Page, other) => {
                return Err(format!(
                    "pages hold only boards, but `{id}` is of type `{other}`"
                ));
            }
            (_, NodeKind::Board) => {
                return Err(format!(
                    "boards sit only on pages, but the board `{id}` is inside the {kind} `{parent}`"
                ));
            }
            (_, NodeKind::Page) => {
                return Err(format!(
                    "pages are never inside other nodes, but the page `{id}` is inside the {kind} `{parent}`"
                ));
            }
            _ => {}
        }
        let placement = child.placement();
        let flow = mode.filter(|mode| *mode != LayoutMode::None && !placement.is_absolute());
        let position = first_set([("x", placement.x.is_some()), ("y", placement.y.is_some())]);
        if let (Some(mode), Some(key)) = (flow, position) {
            return Err(format!(
                "`{id}` flows in the {mode} layout of `{parent}`, which computes its `{key}`: leave `{key}` out, or set `\"absolute\": true` to place it"
            ));
        }
        let grid_lines = first_set([
            ("gridColumn", placement.grid_column.is_some()),
            ("gridRow", placement.grid_row.is_some()),
        ]);
        if let Some(key) = grid_lines.filter(|_| mode != Some(LayoutMode::Grid)) {
            return Err(format!(
                "`{id}` sets `{key}`, but only the children of grid layouts are placed on grid lines"
            ));
        }
        let self_alignment = first_set([
            ("justifySelf", placement.justify_self.is_some()),
            ("alignSelf", placement.align_self.is_some()),
        ]);
        if let Some(key) = self_alignment.filter(|_| mode != Some(LayoutMode::Grid)) {
            return Err(format!(
                "`{id}` sets `{key}`, but only the children of grid layouts align themselves in their grid areas"
            ));
        }
    }
    Ok(())
}

/// Checks that no two nodes of the tree under `root` share an ID.
fn check_unique_ids(root: &Node) -> Result<(), String> {
    let mut seen = HashSet::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if !seen.insert(node.id) {
            return Err(format!(
                "node IDs are unique, but `{}` names more than one node",
                node.id
            ));
        }
        pending.extend(node.children());
    }
    Ok(())
}
