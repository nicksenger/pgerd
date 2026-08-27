//! `pgerd` — render the ERD of a PostgreSQL database as an interactive,
//! pannable/zoomable graph.
//!
//! Usage: `pgerd <postgres-url>`
//!
//! Pass the literal string `debug` instead of a URL to preview the table-node
//! widgets (no database, no layout) — useful for UI development.

mod db;
mod erd;
mod hover;

use iced::widget::{text, Column, Container};
use iced::{keyboard, Alignment, Color, Element, Length, Task};

use crate::db::Schema;
use crate::erd::{Erd, SUGIYAMA_ID};

const WINDOW_WIDTH: f32 = 1400.0;
const WINDOW_HEIGHT: f32 = 900.0;

#[derive(Debug, Clone)]
enum Message {
    SchemaLoaded(Result<Schema, String>),
    /// Collapse or expand a table node.
    ToggleNode(u32),
    /// Expand every table node (the `+` key).
    ExpandAll,
    /// Contract every table node (the `-` key).
    ContractAll,
    /// The cursor entered the node for table `node`.
    NodeHovered(u32),
    /// The cursor left the node for table `node`; only clears the hover if
    /// that node is still the hovered one (enter/leave pairs for different
    /// nodes can arrive out of order in a single cursor move).
    NodeUnhovered(u32),
}

/// Force the sugiyama graph widget to re-render after hover changes.
fn force_review() -> Task<Message> {
    iced_sugiyama::force_review(iced_sugiyama::Id::new(SUGIYAMA_ID))
}

/// Snapshot the currently displayed graph state, prime animation, and rebuild
/// the sugiyama widget after a node is toggled.
fn invalidate() -> Task<Message> {
    iced_sugiyama::invalidate(iced_sugiyama::Id::new(SUGIYAMA_ID))
}

#[derive(Debug, Clone)]
enum Status {
    Loading,
    Error(String),
    Ready(Erd),
}

struct Pgerd {
    status: Status,
    url: String,
    /// When true (the `debug` pseudo-URL) render table nodes in a plain list.
    plain: bool,
}

impl Pgerd {
    fn new(url: &str) -> (Self, Task<Message>) {
        // `debug` previews the table nodes in a plain list; `debug-graph`
        // runs the same sample schema through the full graph layout.
        if url == "debug" || url == "debug-graph" {
            return (
                Pgerd {
                    status: Status::Ready(Erd::build(&db::sample())),
                    url: url.to_string(),
                    plain: url == "debug",
                },
                Task::none(),
            );
        }

        let url_owned = url.to_string();
        let load = Task::perform(async move { db::load(&url_owned) }, Message::SchemaLoaded);

        (
            Pgerd {
                status: Status::Loading,
                url: url.to_string(),
                plain: false,
            },
            load,
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SchemaLoaded(result) => {
                self.status = match result {
                    Ok(schema) => Status::Ready(Erd::build(&schema)),
                    Err(error) => Status::Error(error),
                };
                Task::none()
            }
            Message::ToggleNode(node) => {
                if let Status::Ready(erd) = &mut self.status {
                    erd.toggle_node(node);
                }
                Task::batch([
                    invalidate(),
                    Task::perform(
                        async move { Message::NodeHovered(node) },
                        std::convert::identity,
                    ),
                ])
            }
            // `+` expands every node, `-` contracts them all.
            Message::ExpandAll | Message::ContractAll => {
                let collapse = matches!(message, Message::ContractAll);
                let mut tasks = vec![invalidate()];
                if let Status::Ready(erd) = &mut self.status {
                    erd.set_all_collapsed(collapse);
                    // Keep the hover highlight consistent after the rebuild,
                    // mirroring what a single-node toggle does.
                    if let Some(node) = erd.hovered_node {
                        tasks.push(Task::perform(
                            async move { Message::NodeHovered(node) },
                            std::convert::identity,
                        ));
                    } else {
                        tasks.push(Task::perform(
                            async move { Message::NodeUnhovered(0) },
                            std::convert::identity,
                        ))
                    }
                }
                Task::batch(tasks)
            }
            Message::NodeHovered(node) => {
                if let Status::Ready(erd) = &mut self.status {
                    erd.hovered_node = Some(node);
                }
                force_review()
            }
            Message::NodeUnhovered(node) => {
                if let Status::Ready(erd) = &mut self.status {
                    if erd.hovered_node == Some(node) {
                        erd.hovered_node = None;
                    }
                }
                force_review()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.status {
            Status::Loading => text("Connecting to database…").size(20).into(),
            Status::Error(error) => Column::with_children(vec![
                text("Failed to load schema")
                    .size(24)
                    .color(Color::from_rgb8(235, 110, 110))
                    .into(),
                text(error.clone())
                    .size(14)
                    .color(Color::from_rgb8(180, 190, 200))
                    .into(),
            ])
            .spacing(12)
            .align_x(Alignment::Center)
            .into(),
            Status::Ready(erd) => {
                if self.plain {
                    erd.view()
                } else {
                    erd.view()
                }
            }
        };

        Container::new(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .padding(16)
            .into()
    }

    fn title(&self) -> String {
        // Show only the part after `@` (host/database), hiding credentials.
        let display = self.url.rsplit('@').next().unwrap_or(&self.url);
        format!("pgerd — {}", display)
    }
}

/// Translate a keyboard event into an expand/contract-all message.
///
/// Only plain `+` and `-` presses are handled; anything else (including
/// key releases) is ignored.
fn key_message(event: keyboard::Event) -> Option<Message> {
    if let keyboard::Event::KeyPressed { key, .. } = event {
        match key.as_ref() {
            keyboard::Key::Character("+") => return Some(Message::ExpandAll),
            keyboard::Key::Character("-") => return Some(Message::ContractAll),
            _ => {}
        }
    }
    None
}

fn main() -> iced::Result {
    let mut args = std::env::args().skip(1);
    let url = match args.next() {
        Some(url) if !url.starts_with('-') => url,
        _ => {
            std::process::exit(2);
        }
    };
    if args.next().is_some() {
        std::process::exit(2);
    }

    iced::application(move || Pgerd::new(&url), Pgerd::update, Pgerd::view)
        .title(Pgerd::title)
        .theme(iced::Theme::Dark)
        .window_size(iced::Size::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        // `+` expands all nodes, `-` contracts them all.
        .subscription(|_| iced::keyboard::listen().filter_map(key_message))
        .run()
}
