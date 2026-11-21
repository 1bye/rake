//! Native evaluation app using Rune and Rake's UI-independent crates.

use std::fs;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rake_diagnostics::Collector;
use rake_protocol::router::RouteAddress;
use rake_resources::Resources;
use rune::tok::{self, Theme, motion, radius, space, type_};
use rune::{
    App, AppSpec, Chrome, Cx, El, GrowthSpec, ScrollSpec, TextEdit, TextStyle, col, el, row, text,
    text_input,
};

#[path = "../../activity.rs"]
mod activity;
mod route;

use activity::{APP_TITLE, Activity, WINDOW_HEIGHT, WINDOW_WIDTH};
use route::{Navigation, Route};

const APP_ID: &str = "dev.rake.rune-eval";
const HEADING: TextStyle = TextStyle::new(24.0, 600.0);
const BODY: TextStyle = TextStyle::new(14.0, 400.0);
const ROW_HEIGHT: f32 = 34.0;

struct State {
    navigation: Navigation,
    events: Vec<Activity>,
    diagnostics: Collector,
    show_diagnostics: bool,
    expanded: bool,
    dark: bool,
    notifications: bool,
    workspace_name: TextEdit,
}

impl State {
    fn new(events: Vec<Activity>) -> Self {
        Self {
            navigation: Navigation::default(),
            events,
            diagnostics: Collector::new(32),
            show_diagnostics: std::env::var_os("RAKE_RUNE_DIAGNOSTICS").is_some(),
            expanded: false,
            dark: false,
            notifications: true,
            workspace_name: TextEdit::new("Studio North"),
        }
    }

    fn record_route(&self, address: RouteAddress) {
        self.diagnostics.record(
            "INFO",
            APP_ID,
            "event",
            Vec::new(),
            [("route".into(), address.route.into())].into(),
        );
        if self.show_diagnostics
            && let Some(event) = self.diagnostics.snapshot().records.last()
        {
            eprintln!(
                "navigation {} at {} µs · {}",
                event.sequence, event.elapsed_us, event.fields["route"]
            );
        }
    }

    fn open(&mut self, route: Route) {
        if let Some(address) = self.navigation.open(route) {
            self.record_route(address);
        }
    }
}

fn button(id: &'static str, label: &'static str, theme: Theme) -> El {
    row()
        .key(id)
        .role("button")
        .label(label)
        .focusable(true)
        .h(36.0)
        .px(space::S3)
        .center_y()
        .radius(radius::SM)
        .fill(theme.control_track)
        .hover(theme.hover)
        .focus_fill(theme.control_off)
        .child(text(label).font(BODY).color(theme.text))
}

fn panel(theme: Theme) -> El {
    col()
        .w_full()
        .padding(20.0)
        .gap(16.0)
        .radius(radius::MD)
        .fill(theme.card)
}

fn overview(state: &State, theme: Theme) -> El {
    let label = if state.expanded {
        "Contract bar"
    } else {
        "Expand bar"
    };
    panel(theme)
        .child(text("Motion preview").font(type_::TITLE).color(theme.text))
        .child(
            text("Reverse the bar while it is moving.")
                .font(BODY)
                .color(theme.text_secondary),
        )
        .child(
            row().h(32.0).center_y().child(
                el().key("bar")
                    .size(if state.expanded { 312.0 } else { 72.0 }, 8.0)
                    .radius(radius::SM)
                    .fill(tok::accent(0, state.dark))
                    .motion(motion::CALM),
            ),
        )
        .child(
            button("motion", label, theme)
                .w(128.0)
                .on_press(|s: &mut State| s.expanded = !s.expanded),
        )
}

fn activity(state: &State, theme: Theme) -> El {
    col()
        .key("activity-list")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .radius(radius::MD)
        .fill(theme.card)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .key("activity-rows")
                .growth(GrowthSpec::default())
                .virtual_rows(
                    state.events.len(),
                    ROW_HEIGHT,
                    |s: &State, index| s.events[index].id.as_str().into(),
                    |s: &State, index| {
                        let t = tok::theme(s.dark);
                        row()
                            .h(ROW_HEIGHT)
                            .w_full()
                            .center_y()
                            .px(space::S3)
                            .border_sides([0., 0., 1., 0.], t.separator)
                            .child(
                                text(s.events[index].message.clone())
                                    .font(BODY)
                                    .color(t.text)
                                    .truncate(),
                            )
                    },
                )
                .virtual_rows_version(0),
        )
}

fn settings(state: &State, theme: Theme) -> El {
    let mode_label = if state.dark {
        "Use light theme"
    } else {
        "Use dark theme"
    };
    let switch_text = if state.notifications { "On" } else { "Off" };
    panel(theme)
        .child(text("Workspace name").font(type_::TITLE).color(theme.text))
        .child(
            text_input(&state.workspace_name, BODY)
                .key("workspace-name")
                .label("Workspace name")
                .single_line()
                .on_edit(|s: &mut State| &mut s.workspace_name)
                .w_full()
                .h(36.0)
                .px(space::S3)
                .center_y()
                .radius(radius::SM)
                .fill(theme.bg)
                .border(1., theme.card_border)
                .color(theme.text)
                .caret(theme.text, tok::accent(0, state.dark).alpha(0.4)),
        )
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(text("Notifications").font(BODY).color(theme.text))
                .child(
                    button("notifications", switch_text, theme)
                        .w(64.0)
                        .center()
                        .role("switch")
                        .label(if state.notifications {
                            "Notifications On"
                        } else {
                            "Notifications Off"
                        })
                        .checked(state.notifications)
                        .on_press(|s: &mut State| s.notifications = !s.notifications),
                ),
        )
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(text("Appearance").font(BODY).color(theme.text))
                .child(
                    button("theme", mode_label, theme)
                        .w(144.0)
                        .center()
                        .on_press(|s: &mut State| s.dark = !s.dark),
                ),
        )
}

fn view(state: &State, _: &mut Cx<State>) -> El {
    let theme = tok::theme(state.dark);
    let route = state.navigation.current();
    let (subtitle, content) = match route {
        Route::Overview => (
            "Native rendering and interruptible motion.",
            overview(state, theme),
        ),
        Route::Activity => ("1,200 events · scroll to browse", activity(state, theme)),
        Route::Settings => ("Workspace preferences", settings(state, theme)),
    };
    let mut sidebar = col()
        .key("sidebar")
        .role("navigation")
        .label("Workspace")
        .w(176.0)
        .h_full()
        .padding(16.0)
        .gap(space::S2)
        .fill(theme.card)
        .child(
            row()
                .h(36.0)
                .center_y()
                .px(space::S3)
                .child(text("Workspace").font(type_::TITLE).color(theme.text)),
        );

    for destination in Route::ALL {
        sidebar = sidebar.child(
            button(destination.key(), destination.title(), theme)
                .w_full()
                .on_press_tx(move |s: &mut State, tx| {
                    s.open(destination);
                    tx.instant();
                }),
        );
    }

    row().w_full().h_full().fill(theme.bg).child(sidebar).child(
        col()
            .key(route.key())
            .grow(1.0)
            .min_w(0.0)
            .min_h(0.0)
            .h_full()
            .padding(space::S6)
            .gap(16.0)
            .child(
                text(route.title())
                    .role("heading")
                    .level(1)
                    .font(HEADING)
                    .color(theme.text),
            )
            .child(text(subtitle).font(BODY).color(theme.text_secondary))
            .child(content),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::var_os("RAKE_RUNE_RESOURCES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures"));
    let resources = Resources::load(&root)?;
    let events = activity::activity(&fs::read_to_string(
        resources.path("activity-samples.tsv")?,
    )?)?;
    let state = State::new(events);
    state.record_route(Route::Overview.address());
    rune::set_app_id(APP_ID);

    let mut app = App::new(AppSpec::<State> {
        scenes: &[
            ("activity", |s| s.open(Route::Activity)),
            ("settings", |s| s.open(Route::Settings)),
        ],
        ..AppSpec::new(
            APP_TITLE,
            (WINDOW_WIDTH, WINDOW_HEIGHT),
            || State::new(Vec::new()),
            view,
        )
    })
    .init(|| state)
    .chrome(Chrome {
        full_size_content: false,
        ..Chrome::DEFAULT
    })
    .tab_visits_controls(true);

    if std::env::var_os("RAKE_EVAL_FRAMES").is_some() {
        let started = Instant::now();
        let epoch_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64() * 1000.0;
        app = app.on_frame_encoded(move |frame| {
            if let (Some(at), Some(build)) = (frame.started(), frame.update_ms()) {
                eprintln!(
                    "frame,{:.3},{build:.6}",
                    epoch_ms + at.duration_since(started).as_secs_f64() * 1000.0
                );
            }
        });
    }

    app.try_run().map_err(std::io::Error::other)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    #[test]
    fn native_view_retargets_motion_and_routes_settings_controls() {
        let events =
            activity::activity(include_str!("../../fixtures/activity-samples.tsv")).unwrap();
        let mut rt = Runtime::new(
            State::new(events),
            view,
            (WINDOW_WIDTH, WINDOW_HEIGHT),
            Env {
                system_dark: false,
                scale: 2.0,
            },
        );
        rt.frame(0.0);
        let width = |rt: &Runtime<State>| rt.presented("bar").unwrap().0[2];
        let initial = width(&rt);

        let click = |rt: &mut Runtime<State>, key| {
            let (x, y) = rt.find(key).unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(1.0 / 60.0);
        };
        click(&mut rt, "motion");
        for _ in 0..5 {
            rt.frame(1.0 / 60.0);
        }
        let intermediate = width(&rt);
        assert!(initial < intermediate && intermediate < 312.0);

        click(&mut rt, "motion");
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert!((width(&rt) - initial).abs() < 0.01);

        click(&mut rt, "settings");
        assert_eq!(rt.state.navigation.current(), Route::Settings);
        click(&mut rt, "notifications");
        click(&mut rt, "theme");
        assert!(!rt.state.notifications);
        assert!(rt.state.dark);
        assert_eq!(
            rt.state.diagnostics.snapshot().records[0].fields["route"],
            "settings"
        );
    }
}
