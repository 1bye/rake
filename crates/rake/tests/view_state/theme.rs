//! Covers: ambient theme lifecycle and observable GPUI global updates.
//! Real: theme runtime, App globals and subscriptions.
//! Doubles: GPUI TestPlatform; no native window or GPU required.

use std::cell::Cell;
use std::rc::Rc;

use rake::gpui::{self, AppContext, InputEvent};
use rake::theme::{self, ThemeGlobal};

#[test]
fn themes_coexist_replace_without_clone_and_notify_global_observers() {
    struct Colors(u32);

    struct Metrics(u32);

    let cx = gpui::TestAppContext::single();

    assert!(theme::try_current::<Colors>().is_none());
    cx.update(|cx| {
        theme::install(cx, Colors(1));
        theme::install(cx, Metrics(2));
    });
    let old = theme::current::<Colors>();
    let observed = Rc::new(Cell::new(0));
    let values = observed.clone();

    let _subscription = cx.update(|cx| {
        cx.observe_global::<ThemeGlobal<Colors>>(move |cx| {
            values.set(cx.global::<ThemeGlobal<Colors>>().0.0);
        })
    });

    cx.run_until_parked();
    cx.update(|cx| theme::install(cx, Colors(3)));
    cx.run_until_parked();

    assert_eq!(observed.get(), 3);
    assert_eq!(old.0, 1, "existing handles retain the previous theme");
    assert_eq!(theme::current::<Colors>().0, 3);
    assert_eq!(theme::current::<Metrics>().0, 2);
}

#[test]
fn missing_theme_panic_names_the_type_and_setup_fix() {
    #[derive(Debug)]
    struct NotInstalled;

    let panic = std::panic::catch_unwind(theme::current::<NotInstalled>).unwrap_err();
    let message = panic.downcast_ref::<String>().expect("formatted panic");

    assert!(message.contains("NotInstalled"));
    assert!(message.contains("rake::theme::install(cx, ...)"));
}

#[derive(Clone, rake::Theme)]
pub(super) struct Palette {
    surface: gpui::Rgba,
    hover: gpui::Rgba,
}

#[derive(gpui::IntoElement)]
struct DeepThemed {
    color: Rc<Cell<gpui::Hsla>>,
    background: Rc<std::cell::RefCell<Option<gpui::Fill>>>,
}

impl gpui::RenderOnce for DeepThemed {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl gpui::IntoElement {
        let color = self.color;
        let background = self.background;

        rake::view! {
            <div class="w-20 h-20 bg-surface text-surface hover:bg-hover hover:text-hover"
                style={|mut s| { *background.borrow_mut() = s.style().background.clone(); s }}>
                {gpui::canvas(move |_, window, _| color.set(window.text_style().color), |_, _, _, _| {}).size_full()}
            </div>
        }
    }
}

struct ThemedHost {
    color: Rc<Cell<gpui::Hsla>>,
    background: Rc<std::cell::RefCell<Option<gpui::Fill>>>,
}

impl gpui::Render for ThemedHost {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        rake::view! {
            <div><section><DeepThemed color={self.color.clone()} background={self.background.clone()}/></section></div>
        }
    }
}

#[test]
fn deep_components_read_replaced_themes_and_rebuild_hover_refinements() {
    let mut cx = gpui::TestAppContext::single();
    let color = Rc::new(Cell::new(gpui::Hsla::default()));
    let background = Rc::new(std::cell::RefCell::new(None));

    cx.update(|cx| {
        theme::install(
            cx,
            Palette {
                surface: gpui::rgb(0x112233),
                hover: gpui::rgb(0x445566),
            },
        )
    });
    let window = cx.add_window(|_, _| ThemedHost {
        color: color.clone(),
        background: background.clone(),
    });

    for (surface, hover) in [(0x112233, 0x445566), (0xaabbcc, 0xddeeff)] {
        cx.update(|cx| {
            theme::install(
                cx,
                Palette {
                    surface: gpui::rgb(surface),
                    hover: gpui::rgb(hover),
                },
            )
        });
        // Window refresh schedules a frame; explicit draw uses the real GPUI layout path.
        super::draw(&mut cx, window);

        for coordinate in [500., 10.] {
            cx.update_window(window.into(), |_, window, cx| {
                window.dispatch_event(
                    gpui::MouseMoveEvent {
                        position: gpui::point(gpui::px(coordinate), gpui::px(coordinate)),
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
            })
            .unwrap();
            super::draw(&mut cx, window);
            let expected = if coordinate == 10. { hover } else { surface };

            assert_eq!(color.get(), gpui::rgb_to_hsla(gpui::rgb(expected)));
        }

        assert_eq!(
            format!("{:?}", *background.borrow()),
            format!("{:?}", Some(gpui::Fill::from(gpui::rgb(surface))))
        );
    }
}
