//! Lightweight components own their layout; retained state stays in the counter entities.

use rake::gpui::{self, AnyElement, ElementId, SharedString, prelude::*, rgb};

#[derive(IntoElement)]
pub(super) struct Badge {
    pub id: ElementId,
    pub label: SharedString,
    pub highlighted: bool,
}

impl RenderOnce for Badge {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        rake::view! {
            <span id={self.id} class="text-xs px-2 py-1 rounded-md"
                style={|s| s.bg(if self.highlighted { rgb(0x6b4939) } else { rgb(0x365849) })}>
                {self.label}
            </span>
        }
    }
}

#[derive(IntoElement)]
pub(super) struct Card {
    pub id: ElementId,
    pub header: AnyElement,
    pub footer: Option<AnyElement>,
    pub children: Vec<AnyElement>,
}

impl RenderOnce for Card {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        rake::view! {
            <section id={self.id} class="w-full flex flex-col gap-4 p-6 rounded-xl"
                style={|s| s.max_w(gpui::px(640.)).bg(rgb(0x22323d))}>
                {self.header}
                <div class="flex flex-col gap-4" style={|s| s.children(self.children)}/>
                if let Some(footer) = self.footer {
                    {footer}
                }
            </section>
        }
    }
}

/// Stateless help content retained by the example's owner.
pub(super) struct ActionsHint;

impl Render for ActionsHint {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        rake::view! {
            <p class="px-3 py-2 rounded-md text-sm" style={|s| s.bg(rgb(0x365849)).text_color(rgb(0xf0f4e8))}>
                Actions open an anchored popover; the document browser keeps its selection and virtual-list scroll.
            </p>
        }
    }
}
