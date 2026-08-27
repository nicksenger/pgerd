//! A small wrapper widget that reports when the mouse cursor enters or
//! leaves the bounds of its content.
//!
//! iced 0.14 has no per-widget `on_mouse_enter`/`on_mouse_leave`, so hover is
//! derived from window-level `CursorMoved` events plus a hit test against
//! this widget's own layout bounds (which, inside the sugiyama overlay, are
//! already in the transformed pan/zoom coordinate space).

use iced::advanced::widget::tree;
use iced::advanced::{layout, mouse, renderer, Clipboard, Layout, Shell, Widget};
use iced::{Element, Event, Length, Rectangle, Size};

/// Wraps an element and emits `on_enter`/`on_leave` messages when the cursor
/// crosses its bounds.
pub struct HoverBox<Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'static, Message, Theme, Renderer>,
    on_enter: Message,
    on_leave: Message,
}

impl<Message, Theme, Renderer> HoverBox<Message, Theme, Renderer> {
    pub fn new(
        content: impl Into<Element<'static, Message, Theme, Renderer>>,
        on_enter: Message,
        on_leave: Message,
    ) -> Self {
        Self {
            content: content.into(),
            on_enter,
            on_leave,
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for HoverBox<Message, Theme, Renderer>
where
    Message: Clone + 'static,
    Theme: 'static,
    Renderer: iced::advanced::Renderer + 'static,
{
    fn tag(&self) -> tree::Tag {
        struct Tag;
        tree::Tag::of::<Tag>()
    }

    /// Whether the cursor is currently over this widget.
    fn state(&self) -> tree::State {
        tree::State::new(false)
    }

    fn children(&self) -> Vec<tree::Tree> {
        vec![tree::Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut tree::Tree) {
        tree.diff_children(std::slice::from_ref(&self.content))
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut tree::Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut tree::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );

        let hovered = tree.state.downcast_mut::<bool>();
        match event {
            // Re-check on movement and after releasing a drag (panning moves
            // nodes under a stationary cursor without further movement).
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::ButtonReleased(_)) => {
                let is_over = cursor.is_over(layout.bounds());
                if *hovered != is_over {
                    *hovered = is_over;
                    shell.publish(if is_over {
                        self.on_enter.clone()
                    } else {
                        self.on_leave.clone()
                    });
                }
            }
            // The cursor left the window entirely.
            Event::Mouse(mouse::Event::CursorLeft) if *hovered => {
                *hovered = false;
                shell.publish(self.on_leave.clone());
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &tree::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &tree::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<HoverBox<Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a + 'static,
    Theme: 'a + 'static,
    Renderer: iced::advanced::Renderer + 'a + 'static,
{
    fn from(hover_box: HoverBox<Message, Theme, Renderer>) -> Self {
        Element::new(hover_box)
    }
}
