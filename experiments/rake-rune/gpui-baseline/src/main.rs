//! Native evaluation app on Rake's published GPUI-facing APIs.

use std::cell::OnceCell;
use std::rc::Rc;

use rake::gpui::prelude::*;
use rake::gpui::{
    self, AnyView, Bounds, Context, Entity, TitlebarOptions, Window, WindowBounds, WindowOptions,
    accesskit::{Role, Toggled},
    px, size, uniform_list,
};
use rake::router::{LayoutContext, MountError, PageContext, Route, RouteViews};
use rake::theme::Semantic;
use rake::ui::input::{Input, InputAccessibility, InputState};

#[path = "../../activity.rs"]
mod activity;
mod motion;

use activity::{APP_TITLE, Activity, WINDOW_HEIGHT, WINDOW_WIDTH};

#[cfg(feature = "frame-profile")]
mod profiling;

rake::theme!(Semantic);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, rake::Route)]
enum Destination {
    #[default]
    #[route(id = "overview", path = "/")]
    Overview,
    #[route(id = "activity", path = "/activity")]
    Activity,
    #[route(id = "settings", path = "/settings")]
    Settings,
}

impl Destination {
    const ALL: [Self; 3] = [Self::Overview, Self::Activity, Self::Settings];

    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Activity => "Activity",
            Self::Settings => "Settings",
        }
    }
}

struct Pages {
    events: Rc<[Activity]>,
    // One cached entity per route for this single-window app. Settings survive route changes.
    mounted: [OnceCell<Entity<Page>>; Destination::ALL.len()],
}

impl RouteViews<Destination> for Pages {
    fn mount(
        &self,
        destination: &Destination,
        _: PageContext<Destination>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<AnyView, MountError> {
        let page = self.mounted[*destination as usize].get_or_init(|| {
            if *destination == Destination::Overview {
                motion::initialize(window);
            }
            cx.new(|cx| Page {
                route: *destination,
                events: self.events.clone(),
                expanded: false,
                notifications: true,
                input: (*destination == Destination::Settings).then(|| {
                    cx.new(|cx| {
                        let mut input = InputState::new(window, cx).default_value("Studio North");
                        input.set_accessibility(
                            InputAccessibility::new(Some(Role::TextInput))
                                .with_label(Some("Workspace name".into())),
                        );
                        input
                    })
                }),
            })
        });

        Ok(page.clone().into())
    }
}

struct Shell(LayoutContext<Destination>);

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let navigator = self.0.navigator();
        let outlet = self.0.outlet();

        rake::view! {
            <main
                class="flex flex-row size-full bg-background text-foreground text-sm"
                tab-navigation
            >
                <nav class="flex flex-col w-44 shrink-0 gap-2 p-4 bg-surface">
                    <p class="font-semibold px-3 py-2">"Workspace"</p>
                    for route in Destination::ALL {
                        <button
                            id={route.title()}
                            aria-label={route.title()}

                            class="text-left w-full h-9 px-3 rounded-md bg-muted hover:bg-accent focus-visible:bg-accent"
                            on:click={
                                {
                                    let navigator = navigator.clone();
                                    move |_, _, _| {
                                        if let Ok(tab) = navigator.current_tab() {
                                            drop(tab.replace(route));
                                        }
                                    }
                                }
                            }
                        >
                            {route.title()}
                        </button>
                    }
                </nav>
                <section class="flex-1 h-full min-w-0 overflow-hidden">{outlet}</section>
            </main>
        }
    }
}

struct Page {
    route: Destination,
    events: Rc<[Activity]>,
    expanded: bool,
    notifications: bool,
    input: Option<Entity<InputState>>,
}

impl Page {
    fn overview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = if self.expanded {
            "Contract bar"
        } else {
            "Expand bar"
        };
        let bar = motion::bar(rake::theme::current::<Semantic>().primary);

        rake::view! {
            <div class="flex flex-col gap-4 rounded-lg bg-surface p-5">
                <h2 class="text-base font-semibold">"Motion preview"</h2>
                <p class="text-muted-foreground">"Reverse the bar while it is moving."</p>
                <div class="flex flex-row items-center h-8">{bar}</div>
                <button
                    id="motion"
                    aria-label={label}
                    class="w-32 h-9 px-3 rounded-md bg-muted hover:bg-accent focus-visible:bg-accent"
                    on:click={
                        cx.listener(|page, _, window, cx| {
                            page.expanded = !page.expanded;
                            motion::retarget(page.expanded, window, cx);
                            cx.notify();
                        })
                    }
                >
                    {label}
                </button>
            </div>
        }
    }

    fn activity(&self) -> impl IntoElement {
        let events = self.events.clone();
        let items =
            uniform_list("activity-list", events.len(), move |range, _, _| {
                range.map(|index| {
                let event = &events[index];
                rake::view! {
                    <div
                        id={event.id.clone()}
                        class="flex flex-row w-full h-[34px] items-center border-b border-border px-3 text-sm"
                    >
                        <p class="truncate min-w-0">{event.message.clone()}</p>
                    </div>
                }
            }).collect()
            })
            .size_full();

        rake::view! {
            <div class="flex flex-1 min-h-0 overflow-hidden rounded-lg bg-surface">{items}</div>
        }
    }

    fn settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = rake::theme::current::<Semantic>();
        let dark = palette.appearance == rake::theme::Appearance::Dark;
        let theme_label = if dark {
            "Use light theme"
        } else {
            "Use dark theme"
        };
        let toggled = Toggled::from(self.notifications);
        let switch_label = if self.notifications { "On" } else { "Off" };
        let input = self.input.as_ref().map(Input::new);

        rake::view! {
            <div class="flex flex-col gap-4 rounded-lg bg-surface p-5">
                <p class="font-semibold">"Workspace name"</p>
                <div
                    class="flex flex-row items-center h-9 px-3 rounded-md border border-border bg-background"
                >
                    if let Some(input) = input {
                        {input}
                    }
                </div>
                <div class="flex flex-row items-center justify-between gap-4">
                    <p>"Notifications"</p>
                    <button
                        id="notifications"
                        role={Role::Switch}
                        aria-label={format!("Notifications {switch_label}")}
                        style={move |button| button.aria_toggled(toggled)}
                        class="w-16 h-9 rounded-full bg-muted hover:bg-accent focus-visible:bg-accent"
                        on:click={
                            cx.listener(|page, _, _, cx| {
                                page.notifications = !page.notifications;
                                cx.notify();
                            })
                        }
                    >
                        {switch_label}
                    </button>
                </div>
                <div class="flex flex-row items-center justify-between gap-4">
                    <p>"Appearance"</p>
                    <button
                        id="theme"
                        aria-label={theme_label}
                        class="w-36 h-9 rounded-md bg-muted hover:bg-accent focus-visible:bg-accent"
                        on:click={
                            move |_, _, cx| {
                                rake::theme::install(
                                    cx,
                                    if dark {
                                        Semantic::light()
                                    } else {
                                        Semantic::dark()
                                    }
                                );
                            }
                        }
                    >
                        {theme_label}
                    </button>
                </div>
            </div>
        }
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let subtitle = match self.route {
            Destination::Overview => "Native rendering and interruptible motion.",
            Destination::Activity => "1,200 events · scroll to browse",
            Destination::Settings => "Workspace preferences",
        };
        let body = match self.route {
            Destination::Overview => self.overview(cx).into_any_element(),
            Destination::Activity => self.activity().into_any_element(),
            Destination::Settings => self.settings(cx).into_any_element(),
        };

        rake::view! {
            <div
                class="flex flex-col size-full min-h-0 gap-4 p-6 bg-background text-foreground text-sm"
            >
                <h1 class="text-2xl font-semibold">{self.route.title()}</h1>
                <p class="text-muted-foreground">{subtitle}</p>
                {body}
            </div>
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "frame-profile")]
    let _profiler = profiling::start()?;

    let events = activity::activity(include_str!("../../fixtures/activity-samples.tsv"))?;
    let initial = std::env::args()
        .nth(1)
        .map_or(Ok(Destination::Overview), |uri| Destination::from_uri(&uri))?;

    rake::App::new("dev.rake.gpui-eval")
        .window_options(WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(APP_TITLE.into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: gpui::point(px(180.), px(150.)),
                size: size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)),
            })),
            ..Default::default()
        })
        .workspace_chrome(false)
        .navigation_feedback(false)
        .theme(Semantic::light())
        .routes(Pages {
            events: events.into(),
            mounted: Default::default(),
        })
        .initial(initial)
        .layout(|layout, _, cx| cx.new(|_| Shell(layout)))
        .run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_destinations_round_trip_through_rake() {
        for (route, path) in Destination::ALL
            .into_iter()
            .zip(["/", "/activity", "/settings"])
        {
            assert_eq!(route.to_uri().unwrap(), path);
            assert_eq!(Destination::from_uri(path).unwrap(), route);
        }
        assert!(Destination::from_uri("/missing").is_err());
    }
}
