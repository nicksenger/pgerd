//! Builds the layered graph from a [`Schema`] and renders it with the
//! `iced-sugiyama` widget, using crow's-foot notation for the relationship
//! cardinality glyphs.

use std::collections::HashMap;

use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::widget::text::Wrapping;
use iced::widget::{button, container, text, Column, Container, Row};
use iced::widget::Space;
use iced::{alignment, border, mouse, Background, Color, Element, Font, Length, Point, Rectangle, Shadow, Vector};

use iced_sugiyama::{Config, EdgeEndpointKind, Graph, OutgoingEdgeStyle, Sugiyama};

use crate::db::{Column as DbColumn, Schema, Table};
use crate::hover::HoverBox;

/// The widget id of the animated sugiyama graph; used to target
/// [`iced_sugiyama::force_review`] from the update handlers.
pub const SUGIYAMA_ID: &str = "erd-graph";

// --- typography / sizing constants (monospace keeps width estimates exact) --
const HEADER_FONT: f32 = 14.0;
const BODY_FONT: f32 = 13.0;
const TYPE_FONT: f32 = 12.0;
const LABEL_FONT: f32 = 11.0;

const CHAR_W: f64 = 0.6; // monospace advance as a fraction of font size
const H_PADDING: f64 = 12.0;
const HEADER_H: f64 = 30.0;
const ROW_H: f64 = 25.0;
const INDICATOR_W: f64 = 12.0;
const INDICATOR_GAP: f64 = 9.0;
const NAME_TYPE_GAP: f64 = 14.0;
const TOGGLE_GAP: f64 = 8.0; // gap between the table name and the toggle button
const TOGGLE_SPACE: f64 = 34.0; // estimated width of the header's toggle button area
const TOGGLE_GLYPH: f32 = 16.0; // side length of the expand-glyph canvas

// --- palette (dark, cohesive) ----------------------------------------------
fn surface_bg() -> Color {
    Color::from_rgb8(30, 36, 46) // table body
}
fn surface_border() -> Color {
    Color::from_rgb8(72, 84, 102)
}
fn header_bg() -> Color {
    Color::from_rgb8(45, 92, 63) // Postgres-ish green
}
fn header_text() -> Color {
    Color::from_rgb8(236, 243, 240)
}
fn row_bg_even() -> Color {
    surface_bg()
}
fn row_bg_odd() -> Color {
    Color::from_rgb8(35, 42, 53) // subtle stripe
}
fn pk_color() -> Color {
    Color::from_rgb8(126, 200, 148) // green key
}
fn fk_color() -> Color {
    Color::from_rgb8(126, 176, 226) // blue foreign key
}
fn self_fk_color() -> Color {
    Color::from_rgb8(167, 139, 250) // purple self-referencing foreign key
}
fn col_name_color(is_pk: bool, is_fk: bool, is_self_fk: bool) -> Color {
    if is_pk {
        pk_color()
    } else if is_self_fk {
        self_fk_color()
    } else if is_fk {
        fk_color()
    } else {
        Color::from_rgb8(206, 214, 224)
    }
}
fn type_color() -> Color {
    Color::from_rgb8(126, 139, 152) // muted
}
fn edge_color() -> Color {
    Color::from_rgb8(150, 162, 178) // light, readable on dark bg
}
fn dimmed_edge_alpha() -> f32 {
    0.15
}
fn hover_border() -> Color {
    Color::from_rgb8(196, 212, 228) // brightened table border while hovered
}

/// The fully-built ERD: the layered [`Graph`] plus the per-node table data and
/// display names (both indexed by node id).
#[derive(Clone)]
pub struct Erd {
    pub graph: Graph,
    pub tables: Vec<Table>,
    pub display_names: Vec<String>,
    /// Per-edge label (the FK column names), parallel to `graph.edges`.
    pub edge_labels: Vec<String>,
    /// Per-node collapsed state; `false` means the node is fully expanded.
    /// All nodes start out expanded.
    collapsed: Vec<bool>,
    /// The table currently under the mouse cursor, if any. While set, edges
    /// to/from that table are highlighted and the rest are dimmed.
    pub hovered_node: Option<u32>,
}

/// How an edge relates to the currently hovered table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeHighlight {
    /// No node is hovered; edges keep their default look.
    None,
    /// The edge leaves the hovered table (the FK / "many" side).
    Outgoing,
    /// The edge enters the hovered table (the referenced / "one" side).
    Incoming,
    /// A different node is hovered; this edge is dimmed.
    Dimmed,
}

impl Erd {
    /// Assign a node per table and an edge per foreign key.
    pub fn build(schema: &Schema) -> Self {
        let tables = schema.tables.clone();
        let node_ids: HashMap<(String, String), u32> = tables
            .iter()
            .enumerate()
            .map(|(index, table)| (table.key(), index as u32))
            .collect();

        // Qualify names that are ambiguous across schemas.
        let mut name_counts: HashMap<&str, usize> = HashMap::new();
        for table in &tables {
            *name_counts.entry(table.name.as_str()).or_insert(0) += 1;
        }
        let display_names: Vec<String> = tables
            .iter()
            .map(|table| {
                let count = name_counts.get(table.name.as_str()).copied().unwrap_or(0);
                if count > 1 {
                    format!("{}.{}", table.schema, table.name)
                } else {
                    table.name.clone()
                }
            })
            .collect();

        let nodes: Vec<u32> = (0..tables.len() as u32).collect();
        let mut edges: Vec<(u32, u32)> = Vec::new();
        let mut edge_labels: Vec<String> = Vec::new();
        for fk in &schema.foreign_keys {
            if fk.is_self() {
                // Self-references are shown via the purple column styling
                // instead of an edge back to the same node.
                continue;
            }
            let from = node_ids.get(&(fk.from_schema.clone(), fk.from_table.clone()));
            let to = node_ids.get(&(fk.to_schema.clone(), fk.to_table.clone()));
            if let (Some(&from), Some(&to)) = (from, to) {
                edges.push((from, to));
                edge_labels.push(fk.from_columns.join(", "));
            }
        }

        let config = Config {
            vertex_spacing: 40.0,
            ..Default::default()
        };

        let table_count = tables.len();

        Erd {
            graph: Graph::new(nodes, edges).config(config),
            tables,
            display_names,
            edge_labels,
            collapsed: vec![false; table_count],
            hovered_node: None,
        }
    }

    /// Toggle the collapsed state of a table node.
    pub fn toggle_node(&mut self, node: u32) {
        if let Some(flag) = self.collapsed.get_mut(node as usize) {
            *flag = !*flag;
        }
    }

    /// Set the collapsed state of every table node at once.
    pub fn set_all_collapsed(&mut self, collapsed: bool) {
        for flag in &mut self.collapsed {
            *flag = collapsed;
        }
    }

    /// How `edge` relates to the currently hovered table.
    fn edge_highlight(&self, edge: (u32, u32)) -> EdgeHighlight {
        match self.hovered_node {
            Some(node) if edge.0 == node => EdgeHighlight::Outgoing,
            Some(node) if edge.1 == node => EdgeHighlight::Incoming,
            Some(_) => EdgeHighlight::Dimmed,
            None => EdgeHighlight::None,
        }
    }

    /// The stroke gradient colors for an edge in its current highlight state.
    fn edge_stroke_colors(&self, edge: (u32, u32)) -> (Color, Color) {
        match self.edge_highlight(edge) {
            // Outgoing edges take the FK blue, incoming edges the PK green.
            EdgeHighlight::Outgoing => (fk_color(), fk_color()),
            EdgeHighlight::Incoming => (pk_color(), pk_color()),
            EdgeHighlight::Dimmed => {
                let dim = edge_color().scale_alpha(dimmed_edge_alpha());
                (dim, dim)
            }
            EdgeHighlight::None => (edge_color(), edge_color().scale_alpha(0.45)),
        }
    }

    /// The crow's-foot glyph color for an edge in its current highlight state.
    fn endpoint_glyph_color(&self, edge: (u32, u32)) -> Color {
        match self.edge_highlight(edge) {
            EdgeHighlight::Outgoing => fk_color(),
            EdgeHighlight::Incoming => pk_color(),
            EdgeHighlight::Dimmed => edge_color().scale_alpha(dimmed_edge_alpha()),
            EdgeHighlight::None => edge_color(),
        }
    }

    /// Estimated node size in layout units (used before measurement refines it).
    pub fn node_size(&self, node: u32) -> (f64, f64) {
        let table = &self.tables[node as usize];
        // Reserve room for the header's toggle button next to the name.
        let name_width =
            est_text_width(&self.display_names[node as usize], HEADER_FONT) + TOGGLE_SPACE;

        let mut content_width = name_width;
        for column in &table.columns {
            let width = INDICATOR_W
                + INDICATOR_GAP
                + est_text_width(&column.name, BODY_FONT)
                + NAME_TYPE_GAP
                + est_text_width(&column.data_type, TYPE_FONT);
            content_width = content_width.max(width);
        }

        let width = (content_width + H_PADDING * 2.0).max(150.0);
        let height = if self.collapsed[node as usize] {
            HEADER_H
        } else {
            HEADER_H + table.columns.len() as f64 * ROW_H + 8.0
        };
        (width, height)
    }

    /// The iced widget shown for a table node.
    pub fn table_node(&self, node: u32) -> Element<'static, crate::Message> {
        let table = &self.tables[node as usize];
        let name = self.display_names[node as usize].clone();
        let collapsed = self.collapsed[node as usize];
        // A deterministic width keeps the `Fill` rows below well-defined and
        // matches the size the graph layout is computed with.
        let width = self.node_size(node).0.max(150.0) as f32;
        let hovered = self.hovered_node == Some(node);

        // Toggle button: a simple "-" while expanded, a 45° double-headed
        // arrow while collapsed (click to expand again).
        let icon: Element<'static, crate::Message> = if collapsed {
            canvas::Canvas::new(ExpandGlyph { color: header_text() })
                .width(TOGGLE_GLYPH)
                .height(TOGGLE_GLYPH)
                .into()
        } else {
            text("-").size(HEADER_FONT).font(Font::MONOSPACE).into()
        };

        let toggle = button(icon)
            .on_press(crate::Message::ToggleNode(node))
            .padding([2.0, 4.0])
            .style(|_theme: &iced::Theme, status: button::Status| {
                let background = match status {
                    button::Status::Hovered => {
                        Some(Background::Color(Color::from_rgba8(255, 255, 255, 0.12)))
                    }
                    button::Status::Pressed => {
                        Some(Background::Color(Color::from_rgba8(255, 255, 255, 0.22)))
                    }
                    _ => None,
                };
                button::Style {
                    background,
                    text_color: header_text(),
                    border: border::rounded(4.0),
                    shadow: Shadow::default(),
                    ..Default::default()
                }
            });

        // The name is left-aligned; the toggle button sits at the top-right.
        let header = container(
            Row::with_children(vec![
                text(name)
                    .size(HEADER_FONT)
                    .font(Font::MONOSPACE)
                    .wrapping(Wrapping::None)
                    .width(Length::Fill)
                    .into(),
                toggle.into(),
            ])
            .spacing(TOGGLE_GAP as f32)
            .align_y(alignment::Vertical::Center),
        )
        .width(Length::Fill)
        .padding([7.0, H_PADDING as f32])
        .style(|_| header_style());

        let mut children: Vec<Element<'static, crate::Message>> = vec![header.into()];
        if !collapsed {
            for (index, column) in table.columns.iter().enumerate() {
                children.push(column_row(column, index % 2 == 1));
            }
        }

        let body = Column::with_children(children).spacing(0);

        let node_element: Element<'static, crate::Message> = Container::new(body)
            .width(Length::Fixed(width))
            .style(move |_| table_style(hovered))
            .into();

        // Report hover so the edges to/from this table can be highlighted.
        HoverBox::new(
            node_element,
            crate::Message::NodeHovered(node),
            crate::Message::NodeUnhovered(node),
        )
        .into()
    }

    /// The crow's-foot endpoint glyph for an edge end.
    pub fn endpoint_glyph(
        &self,
        kind: EdgeEndpointKind,
        angle_radians: f32,
        color: Color,
    ) -> Element<'static, crate::Message> {
        let (cardinality, angle) = match kind {
            // The FK / child side is the "many" end.
            EdgeEndpointKind::Source => (Cardinality::Many, angle_radians),
            // The referenced / parent side is the "one" end.
            EdgeEndpointKind::Destination => (Cardinality::One, angle_radians + std::f32::consts::PI),
        };

        let glyph = CrowFootGlyph {
            cardinality,
            color,
            angle,
        };
        canvas::Canvas::new(glyph).width(48.0).height(48.0).into()
    }

    /// Assemble the animated [`Sugiyama`] widget for this ERD.
    pub fn view(&self) -> Element<'_, crate::Message> {
        let sugiyama: Sugiyama<crate::Message, iced::Theme, iced::Renderer> =
            Sugiyama::new(&self.graph, |node| self.table_node(node))
                .id(iced_sugiyama::Id::new(SUGIYAMA_ID))
                .layout_fn(iced_sugiyama::microdot_layout)
                .node_size(|node| self.node_size(node))
                .measure_node_sizes(true)
                .stroke_width(1.8)
                .edge_color(|ctx| self.edge_stroke_colors(ctx.edge))
                // Give highlighted edges a little extra weight.
                .outgoing_edge_style(|ctx| match self.edge_highlight(ctx.edge) {
                    EdgeHighlight::Outgoing | EdgeHighlight::Incoming => OutgoingEdgeStyle {
                        width_scale: 1.5,
                        ..OutgoingEdgeStyle::default()
                    },
                    _ => OutgoingEdgeStyle::default(),
                })
                .edge_label(|index, _| self.edge_labels.get(index).cloned())
                .edge_label_element(|index, edge, _text| {
                    let highlight = self.edge_highlight(edge);
                    self.edge_labels
                        .get(index)
                        .filter(|label| !label.is_empty())
                        .map(|label| edge_label_pill(label.clone(), highlight))
                })
                .edge_endpoint(|_index, edge, kind, endpoint| {
                    Some(self.endpoint_glyph(
                        kind,
                        endpoint.angle_radians(),
                        self.endpoint_glyph_color(edge),
                    ))
                })
                .padding(70)
                .auto_fit(iced_sugiyama::AutoFit::Initial(1.0));

        sugiyama.into()
    }
}

impl std::fmt::Debug for Erd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Erd")
            .field("tables", &self.tables)
            .field("display_names", &self.display_names)
            .field("edges", &self.graph.edges.len())
            .finish()
    }
}

fn est_text_width(s: &str, font_size: f32) -> f64 {
    s.chars().count() as f64 * CHAR_W * font_size as f64
}

/// One column row inside a table node.
fn column_row(column: &DbColumn, striped: bool) -> Element<'static, crate::Message> {
    let indicator = if column.is_primary_key {
        key_square(pk_color(), true)
    } else if column.is_self_foreign_key {
        key_square(self_fk_color(), false)
    } else if column.is_foreign_key {
        key_square(fk_color(), false)
    } else {
        container(Space::new().width(INDICATOR_W as f32).height(INDICATOR_W as f32))
            .width(INDICATOR_W as f32)
            .height(INDICATOR_W as f32)
            .into()
    };

    let name = text(column.name.clone())
        .size(BODY_FONT)
        .font(Font::MONOSPACE)
        .color(col_name_color(
            column.is_primary_key,
            column.is_foreign_key,
            column.is_self_foreign_key,
        ))
        .width(Length::Fill);

    let data_type = text(column.data_type.clone())
        .size(TYPE_FONT)
        .font(Font::MONOSPACE)
        .color(type_color());

    let row = Row::with_children(vec![indicator, name.into(), data_type.into()])
        .spacing(INDICATOR_GAP as f32)
        .align_y(alignment::Vertical::Center);

    container(row)
        .width(Length::Fill)
        .padding([4.0, H_PADDING as f32])
        .style(move |_| row_style(striped))
        .into()
}

/// A small rounded indicator: filled for primary keys, hollow for foreign keys.
fn key_square(color: Color, filled: bool) -> Element<'static, crate::Message> {
    let style = move |_: &iced::Theme| {
        let mut s = ContainerStyle::default();
        if filled {
            s.background = Some(color.into());
        } else {
            s.border = border::rounded(3.0).width(1.5).color(color);
        }
        s
    };

    container(Space::new().width(INDICATOR_W as f32).height(INDICATOR_W as f32))
        .width(INDICATOR_W as f32)
        .height(INDICATOR_W as f32)
        .style(style)
        .into()
}

/// A small pill shown at the midpoint of a relationship edge, naming the FK.
fn edge_label_pill(label: String, highlight: EdgeHighlight) -> Element<'static, crate::Message> {
    let (text_color, border_color) = match highlight {
        // Match the highlighted edge: outgoing blue, incoming green.
        EdgeHighlight::Outgoing => (fk_color(), fk_color()),
        EdgeHighlight::Incoming => (pk_color(), pk_color()),
        EdgeHighlight::Dimmed => {
            let dim = Color::from_rgb8(176, 188, 202).scale_alpha(dimmed_edge_alpha());
            (dim, surface_border().scale_alpha(dimmed_edge_alpha()))
        }
        EdgeHighlight::None => (
            Color::from_rgb8(176, 188, 202),
            surface_border(),
        ),
    };

    container(text(label).size(LABEL_FONT).font(Font::MONOSPACE).color(text_color))
        .padding([2.0, 6.0])
        .style(move |_: &iced::Theme| {
            ContainerStyle::default()
                .background(Color::from_rgba8(24, 29, 38, 0.85))
                .border(border::rounded(5.0).width(1.0).color(border_color))
        })
        .into()
}

// --- styles ----------------------------------------------------------------

type ContainerStyle = iced::widget::container::Style;

fn table_style(hovered: bool) -> ContainerStyle {
    let border = if hovered {
        border::rounded(8.0).width(1.5).color(hover_border())
    } else {
        border::rounded(8.0).width(1.0).color(surface_border())
    };

    ContainerStyle::default()
        .background(surface_bg())
        .border(border)
        .shadow(Shadow {
            color: Color::from_rgba8(0, 0, 0, 0.5),
            offset: Vector::new(0.0, 6.0),
            blur_radius: 24.0,
        })
}

fn header_style() -> ContainerStyle {
    ContainerStyle::default().background(header_bg()).color(header_text())
}

fn row_style(striped: bool) -> ContainerStyle {
    ContainerStyle::default().background(if striped {
        row_bg_odd()
    } else {
        row_bg_even()
    })
}

// --- crow's-foot glyph ------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Cardinality {
    /// Exactly one: a single perpendicular tick.
    One,
    /// Many (crow's foot): three prongs fanning toward the node.
    Many,
}

#[derive(Debug, Clone, Copy)]
struct CrowFootGlyph {
    cardinality: Cardinality,
    color: Color,
    /// Rotation such that the local +x axis points *out* of the node, along
    /// the relationship edge (away from the node boundary).
    angle: f32,
}

impl<Message, Theme, Renderer> canvas::Program<Message, Theme, Renderer> for CrowFootGlyph
where
    Renderer: iced::advanced::graphics::geometry::Renderer,
{
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        let center = frame.center();

        frame.with_save(|frame| {
            frame.translate(Vector::new(center.x, center.y));
            frame.rotate(self.angle);

            let stroke = Stroke::default()
                .with_color(self.color)
                .with_width(2.0)
                .with_line_cap(canvas::LineCap::Round);

            match self.cardinality {
                Cardinality::Many => {
                    // Heel sits out on the relationship line; the three prong
                    // tips fan back toward the node boundary.
                    let heel = Point::new(16.0, 0.0);
                    for tip in [
                        Point::new(2.0, -10.0),
                        Point::new(2.0, 0.0),
                        Point::new(2.0, 10.0),
                    ] {
                        frame.stroke(&Path::line(heel, tip), stroke);
                    }
                }
                Cardinality::One => {
                    // A single tick crossing the relationship line.
                    frame.stroke(
                        &Path::line(Point::new(9.0, -8.0), Point::new(9.0, 8.0)),
                        stroke,
                    );
                }
            }
        });

        vec![frame.into_geometry()]
    }
}

/// The maximize/expand glyph shown on collapsed nodes: a 45° diagonal line
/// with arrowheads on both ends.
#[derive(Debug, Clone, Copy)]
struct ExpandGlyph {
    color: Color,
}

impl<Message, Theme, Renderer> canvas::Program<Message, Theme, Renderer> for ExpandGlyph
where
    Renderer: iced::advanced::graphics::geometry::Renderer,
{
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        let center = frame.center();

        frame.with_save(|frame| {
            // Point the local +x axis along the 45° diagonal (up and to the
            // right in screen coordinates).
            frame.translate(Vector::new(center.x, center.y));
            frame.rotate(-std::f32::consts::FRAC_PI_4);

            let stroke = Stroke::default()
                .with_color(self.color)
                .with_width(1.8)
                .with_line_cap(canvas::LineCap::Round);

            // Shaft from lower-left to upper-right (local coordinates).
            frame.stroke(
                &Path::line(Point::new(-7.0, 0.0), Point::new(7.0, 0.0)),
                stroke,
            );

            // Arrowheads: two barbs sweeping back from each tip.
            for (tip, base) in [(7.0_f32, 2.5_f32), (-7.0_f32, -2.5_f32)] {
                frame.stroke(&Path::line(Point::new(tip, 0.0), Point::new(base, 2.6)), stroke);
                frame.stroke(&Path::line(Point::new(tip, 0.0), Point::new(base, -2.6)), stroke);
            }
        });

        vec![frame.into_geometry()]
    }
}
