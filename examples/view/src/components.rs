//! Lightweight components own their layout; retained state stays in the counter entities.

use rake::gpui::{self, AnyElement, ElementId, SharedString, prelude::*};

#[derive(IntoElement)]
pub(super) struct Badge {
    pub id: ElementId,
    pub label: SharedString,
    pub highlighted: bool,
}

impl RenderOnce for Badge {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        rake::view! {
            <span id={self.id} class={if self.highlighted { "text-xs px-2 py-1 rounded-md bg-warning" } else { "text-xs px-2 py-1 rounded-md bg-accent" }}>
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
            <section id={self.id} class="w-full flex flex-col gap-4 p-6 rounded-xl max-w-[640px] bg-panel">
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
            <p class="px-3 py-2 rounded-md text-sm bg-accent text-ink">
                Actions open an anchored popover; the document browser keeps its selection and virtual-list scroll.
            </p>
        }
    }
}
