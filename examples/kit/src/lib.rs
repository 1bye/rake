//! Optional Kit controls inside retained Rake routes, sharing one semantic palette.
#![cfg(all(feature = "kit", not(target_family = "wasm")))]

use rake::gpui::{self, AnyView, Context, FocusHandle, Window, prelude::*};
use rake::inspection::InspectElement as _;

use rake::kit::component::{
    WindowExt as _,
    button::{Button as KitButton, ButtonVariants as _},
    table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow},
};

use rake::prelude::*;
use rake::router::{MountError, OpenTarget, PageContext};
use rake::theme::{Appearance, Semantic};

rake::theme!(Semantic);

#[derive(Clone, Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "components", path = "/components")]
    Components,
}

struct Pages;

impl RouteViews<Destination> for Pages {
    fn mount(
        &self,
        destination: &Destination,
        page: PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<AnyView, MountError> {
        let components = matches!(destination, Destination::Components);

        Ok(cx
            .new(|cx| Demo {
                page,
                components,
                count: 0,
                increment_focus: cx.focus_handle(),
            })
            .into())
    }
}

struct Demo {
    page: PageContext<Destination>,
    components: bool,
    count: usize,
    increment_focus: FocusHandle,
}

fn switch_theme(_: &gpui::ClickEvent, _: &mut Window, cx: &mut gpui::App) {
    let next = if rake::theme::current::<Semantic>().appearance == Appearance::Dark {
        Semantic::light()
    } else {
        Semantic::dark()
    };

    rake::theme::install(cx, next);
}

fn token_table(palette: &Semantic) -> Table {
    Table::new()
        .accessibility_label("Shared semantic roles")
        .bg(palette.surface)
        .text_color(palette.surface_foreground)
        .child(
            TableHeader::new()
                .bg(palette.muted)
                .text_color(palette.muted_foreground)
                .child(
                    TableRow::new()
                        .child(TableHead::new().child("Base / Rake"))
                        .child(TableHead::new().child("Kit")),
                ),
        )
        .child(
            TableBody::new().children(
                [
                    ("surface", "popover"),
                    ("destructive", "danger"),
                    ("primary", "primary"),
                ]
                .into_iter()
                .map(|(base, kit)| {
                    TableRow::new()
                        .child(TableCell::new().child(base))
                        .child(TableCell::new().child(kit))
                }),
            ),
        )
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let navigator = self.page.navigator();
        let palette = rake::theme::current::<Semantic>();

        let button = KitButton::new("kit-increment")
            .primary()
            .label(format!("Kit count: {}", self.count))
            .track_focus(&self.increment_focus)
            .on_click(cx.listener(|this, _, _, cx| {
                this.count += 1;
                cx.notify();
            }))
            .inspect("kit.increment")
            .focus_handle(&self.increment_focus)
            .inspect_handler("click");

        let dialog =
            KitButton::new("kit-dialog-trigger")
                .label("Open Kit dialog")
                .on_click(|_, window, cx| {
                    window.open_dialog(cx, |dialog, _, _| {
                        dialog.title("One Semantic theme").child(rake::view! {
                            <section id="kit-dialog-content" class="gap-3">
                                <p class="text-foreground">
                                    "This dialog is owned by Kit's window presentation layer."
                                </p>
                                <button
                                    id="dialog-theme"
                                    class="px-3 py-2 rounded-md bg-primary text-primary-foreground"
                                    on:click={switch_theme}
                                >
                                    "Switch shared theme"
                                </button>
                            </section>
                        }.inspect("kit.dialog.content"))
                    })
                })
                .inspect("kit.dialog.trigger")
                .inspect_handler("click");

        let table = token_table(&palette).inspect("kit.table");

        rake::view! {
            <section
                id="kit-page"
                class="size-full flex flex-col p-6 gap-4 bg-background text-foreground"
            >
                <h1>"Rake + GPUI Kit"</h1>
                <p>
                    "Routed tabs retain their count. Rake views and Kit controls use one Semantic palette."
                </p>
                <div class="flex flex-row gap-3">
                    <button
                        id="open-components"
                        class="px-3 py-2 rounded-md bg-secondary text-secondary-foreground"
                        on:click={
                            move |_, _, _| {
                                drop(navigator.open(Destination::Components, OpenTarget::NewTab));
                            }
                        }
                    >
                        "Open components tab"
                    </button>
                    <button
                        id="switch-theme"
                        class="px-3 py-2 rounded-md bg-secondary text-secondary-foreground"
                        on:click={switch_theme}
                    >
                        "Switch shared theme"
                    </button>
                </div>
                if self.components {
                    <div id="kit-buttons" class="flex flex-row gap-3">{button}{dialog}</div>
                    <div id="kit-table-frame" class="w-full">{table}</div>
                    <p>
                        "Kit boundaries expose geometry, construction sites and explicit focus; their private descendants remain opaque."
                    </p>
                } else {
                    <p>"Open a components tab to try Kit's button, dialog and table."</p>
                }
            </section>
        }
    }
}

#[rake::app]
fn app() -> App<Destination> {
    App::new("com.example.rake.kit")
        .control(rake::control::ControlConfig::FromEnvironment)
        .theme(Semantic::light())
        .routes(Pages)
        .initial(Destination::Home)
}
