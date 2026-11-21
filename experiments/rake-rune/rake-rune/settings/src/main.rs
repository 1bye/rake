//! Settings and sidebar built with shared Rake components on Rune.
use rake_rune::view;
use rune::{App, AppSpec, Cx, El, TextEdit, text, text_input, tok};

mod backend {
    pub type Element = rune::El;
    pub type Children = rake_rune::Children;
    pub type Handler = fn(&mut super::State);
    pub type Signal = rake_rune::Signal<super::State>;
    pub type Theme = rune::tok::Theme;
    pub fn finish(element: Element) -> Element {
        element
    }
    pub fn slot(children: Children) -> Element {
        rune::col().gap(16.).children(children)
    }
    pub fn finish_button(element: Element, checked: Option<bool>) -> Element {
        match checked {
            Some(checked) => element.role("switch").checked(checked),
            None => element,
        }
    }
}
#[path = "../../../shared/components.rs"]
mod components;
use components::{ButtonProps, button, panel, setting_row, switch};

struct State {
    workspace_name: TextEdit,
    notifications: bool,
    dark: bool,
    destination: &'static str,
}
impl Default for State {
    fn default() -> Self {
        Self {
            workspace_name: TextEdit::new("Studio North"),
            notifications: true,
            dark: false,
            destination: "Settings",
        }
    }
}

fn view(state: &State, _: &mut Cx<State>) -> El {
    let theme = tok::theme(state.dark);
    let input = text_input(&state.workspace_name, tok::type_::BODY)
        .key("workspace-name")
        .label("Workspace name")
        .single_line()
        .on_edit(|s: &mut State| &mut s.workspace_name)
        .w_full()
        .h(36.0)
        .px(tok::space::S3)
        .fill(theme.bg)
        .color(theme.text);
    let notifications = switch(
        theme,
        rake_rune::Signal {
            checked: state.notifications,
            toggle: |s: &mut State| s.notifications = !s.notifications,
        },
    );
    let appearance = button(
        theme,
        ButtonProps {
            id: "theme",
            label: if state.dark {
                "Use light theme"
            } else {
                "Use dark theme"
            },
            on_click: |s| s.dark = !s.dark,
        },
    );
    let settings = panel(
        theme,
        vec![
            text("Workspace name")
                .font(tok::type_::TITLE)
                .color(theme.text),
            input,
            setting_row(theme, "Notifications", vec![notifications]),
            setting_row(theme, "Appearance", vec![appearance]),
        ],
    );
    let destinations: [(&str, backend::Handler); 3] = [
        ("Overview", |s| s.destination = "Overview"),
        ("Activity", |s| s.destination = "Activity"),
        ("Settings", |s| s.destination = "Settings"),
    ];
    view! {
        <main class="flex flex-row size-full bg-background text-foreground">
            <nav
                id="sidebar"
                role="navigation"
                aria-label="Workspace"
                class="flex flex-col w-44 h-full p-4 gap-2 bg-surface"
            >
                <p>"Workspace"</p>
                for (label, on_click) in destinations {
                    {
                        button(
                            theme,
                            ButtonProps {
                                id: label,
                                label,
                                on_click
                            }
                        )
                    }
                }
            </nav>
            <section class="flex flex-col flex-1 p-6 gap-4">
                <h1>{state.destination}</h1>
                if state.destination == "Settings" {
                    {settings}
                } else {
                    <p>"Select Settings for workspace preferences."</p>
                }
            </section>
        </main>
    }
}

fn main() -> Result<(), String> {
    App::new(AppSpec::new(
        "Rake Rune · shared Settings",
        (960.0, 640.0),
        State::default,
        view,
    ))
    .chrome(rune::Chrome {
        full_size_content: false,
        ..rune::Chrome::DEFAULT
    })
    .try_run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    #[test]
    fn shared_settings_render_and_queue_state_changes() {
        let mut runtime = Runtime::new(
            State::default(),
            view,
            (960., 640.),
            Env {
                system_dark: false,
                scale: 1.,
            },
        );
        runtime.frame(0.);
        assert!(runtime.find("workspace-name").is_some());
        let click = |runtime: &mut Runtime<State>, key| {
            let (x, y) = runtime.find(key).expect("shared control exists");
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(1. / 60.);
        };
        click(&mut runtime, "notifications");
        click(&mut runtime, "theme");
        assert!(!runtime.state.notifications);
        assert!(runtime.state.dark);
        click(&mut runtime, "Overview");
        assert_eq!(runtime.state.destination, "Overview");
        assert!(runtime.find("workspace-name").is_none());
        click(&mut runtime, "Settings");
        assert!(runtime.find("workspace-name").is_some());
    }

    #[test]
    fn empty_shared_panel_renders_headlessly() {
        fn empty(_: &(), _: &mut Cx<()>) -> El {
            panel(tok::theme(false), vec![]).key("empty")
        }
        let mut runtime = Runtime::new(
            (),
            empty,
            (200., 100.),
            Env {
                system_dark: false,
                scale: 1.,
            },
        );
        runtime.frame(0.);
        assert!(runtime.find("empty").is_some());
    }

    #[test]
    fn component_props_children_and_match_lower_to_real_elements() {
        struct Panel {
            title: &'static str,
            children: rake_rune::Children,
        }
        impl rake_rune::Component for Panel {
            fn render(self) -> El {
                panel(tok::theme(false), self.children).key(self.title)
            }
        }
        fn tree(_: &(), _: &mut Cx<()>) -> El {
            view! {
                <Panel title="component">
                    match true {
                        true => {
                            <div id="match-child" class="size-4" />
                        }
                        false => {
                            <div id="unreachable" />
                        }
                    }
                </Panel>
            }
        }
        let mut runtime = Runtime::new(
            (),
            tree,
            (200., 100.),
            Env {
                system_dark: false,
                scale: 1.,
            },
        );
        runtime.frame(0.);
        assert!(runtime.find("component").is_some());
        assert!(runtime.find("match-child").is_some());
        assert!(runtime.find("unreachable").is_none());
    }
}
