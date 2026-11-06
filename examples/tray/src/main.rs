//! Two independent tray items, native menus, popup content and repeatable main window.
use rake::{
    gpui::{self, AppContext, Context, Window, prelude::*},
    prelude::*,
    tray::*,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "tray.increment", capability = Headless)]
struct Increment;
#[derive(Default)]
struct Model {
    progress: Cell<u32>,
    mounts: Cell<u32>,
    block_quit: Cell<bool>,
    events: RefCell<Vec<String>>,
}
struct Panel {
    model: Rc<Model>,
    runtime: rake::runtime::RuntimeContext,
}
impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let capabilities = self
            .runtime
            .trays()
            .map(|trays| format!("{:?}", trays.capabilities().backend()))
            .unwrap_or_else(|error| error.to_string());
        gpui::div().size_full().bg(gpui::rgb(0x14202b)).text_color(gpui::rgb(0xf1f5f9)).p_6().flex().flex_col().gap_3()
            .child("Rake tray workbench")
            .child(format!("Backend: {capabilities} · Progress: {}%", self.model.progress.get()))
            .child(gpui::div().id("increment").cursor_pointer().child("Increment progress / update icon").on_click(cx.listener(|this, _, _, cx| {
                let _ = this.runtime.client().and_then(|client| client.execute(Increment).map_err(|_| rake::router::OperationError::InvalidOptions));
                cx.notify();
            })))
            .child(gpui::div().id("transient").cursor_pointer().child("Create/remove a transient tray").on_click(cx.listener(|this, _, _, cx| {
                if let Ok(trays) = this.runtime.trays() {
                    if let Ok(item) = trays.get("transient") { drop(item.remove()); }
                    else if let Ok(image) = icon([60,200,145,255]) { drop(trays.create(Tray::new("transient").icon(image).tooltip("Transient item — click button again to remove"))); }
                }
                cx.notify();
            })))
            .child(gpui::div().id("guard").cursor_pointer().child(format!("Block guarded Quit: {} (toggle)", this_guard(&self.model))).on_click(cx.listener(|this, _, _, cx| { this.model.block_quit.set(!this.model.block_quit.get()); cx.notify(); })))
            .child(gpui::div().id("animate").cursor_pointer().child("Start bounded icon animation").on_click(cx.listener(|this, _, _, cx| {
                let result = (|| {
                    let base = icon([70,155,245,255])?;
                    let frames = vec![base.clone(), base.progress(0.33,0.12,[255;4])?, base.progress(0.66,0.12,[255;4])?];
                    animate("main", Animation::new(frames, std::time::Duration::from_millis(150))?, cx)
                })();
                if let Err(error) = result { this.model.events.borrow_mut().push(error.to_string()); }
                cx.notify();
            })))
            .child(gpui::div().id("stop-animation").cursor_pointer().child("Stop animation").on_click(|_, _, cx| { let _ = stop_animation("main", cx); }))
            .child(self.runtime.trays().map(|trays| format!("Geometry: {:?} · Inline text: {:?} · Drop target: {:?}", trays.capabilities().support(Feature::Rectangle), trays.capabilities().support(Feature::InlineText), trays.capabilities().support(Feature::DropTarget))).unwrap_or_else(|error| error.to_string()))
            .child("Close this window, then choose Open from the tray. The runtime and progress survive.")
            .child(format!("Recent event: {}", self.model.events.borrow().last().map(String::as_str).unwrap_or("none")))
    }
}
fn this_guard(model: &Model) -> bool {
    model.block_quit.get()
}
fn icon(color: [u8; 4]) -> Result<Image, Error> {
    fn raster(size: u32, color: [u8; 4]) -> Result<ImageRepresentation, Error> {
        let mut pixels = vec![0; (size * size * 4) as usize];
        let radius = f64::from(size) * 0.38;
        for y in 0..size {
            for x in 0..size {
                if (f64::from(x) + 0.5 - f64::from(size) / 2.0)
                    .hypot(f64::from(y) + 0.5 - f64::from(size) / 2.0)
                    < radius
                {
                    pixels[(y * size + x) as usize * 4..(y * size + x) as usize * 4 + 4]
                        .copy_from_slice(&color);
                }
            }
        }
        ImageRepresentation::rgba(size, size, pixels)
    }
    Image::new(raster(18, color)?)
        .representation(raster(36, color)?)?
        .representation(raster(32, color)?)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = Rc::new(Model::default());
    let base = icon([70, 155, 245, 255])?;
    let command_model = model.clone();
    let command_image = base.clone();
    let guard = model.clone();
    let events = model.clone();
    let content = model.clone();
    let popup_model = model.clone();
    let menu = Menu::new()
        .item(MenuItem::new("open", "Open window"))
        .item(MenuItem::new("increment", "Increment progress"))
        .item(
            MenuItem::new("options", "Options").kind(ItemKind::Submenu(
                Menu::new()
                    .item(MenuItem::new("checked", "Checked").kind(ItemKind::Check(CheckState::On)))
                    .item(MenuItem::new("radio-a", "Mode A").kind(ItemKind::Radio {
                        group: "mode".into(),
                        selected: true,
                    }))
                    .item(MenuItem::new("radio-b", "Mode B").kind(ItemKind::Radio {
                        group: "mode".into(),
                        selected: false,
                    })),
            )),
        )
        .item(MenuItem::separator("divider"))
        .item(MenuItem::quit("quit", "Quit"));
    let app = App::new("dev.rake.tray-example")
        .tray(
            Tray::new("main")
                .icon(base)
                .tooltip("Rake — native menu")
                .menu(menu),
        )
        .tray(
            Tray::new("status")
                .icon(icon([245, 170, 65, 255])?)
                .tooltip("Rake — rich popup")
                .primary(Interaction::Popup),
        )
        .tray_popup(
            "status",
            Popup::new(340.0, 180.0, move |_, cx| {
                let text = format!("Current progress: {}%", popup_model.progress.get());
                Ok(cx.new(|_| PopupContent(text)).into())
            }),
        )
        .command(move |_: Increment, context| {
            let model = command_model.clone();
            let image = command_image.clone();
            async move {
                let value = (model.progress.get() + 10) % 110;
                model.progress.set(value);
                if let Ok(trays) = context.services.get::<Collection>()
                    && let Ok(item) = trays.get("main")
                {
                    if let Ok(icon) =
                        image.progress(value as f32 / 100.0, 0.12, [255, 255, 255, 255])
                    {
                        drop(item.set_icon(icon));
                    }
                    drop(item.set_tooltip(&format!("Progress: {value}%")));
                    #[cfg(target_os = "macos")]
                    if let Ok(spec) = item.specification() {
                        drop(item.update(spec.macos(macos::Appearance {
                            title: format!("{value}%"),
                            ..Default::default()
                        })));
                    }
                }
                Ok(())
            }
        })?
        .tray_command("main", "increment", Increment)
        .on_tray_event(move |event, cx| {
            let mut history = events.events.borrow_mut();
            if history.len() == 32 {
                history.remove(0);
            }
            history.push(format!("{}: {:?}", event.id, event.kind));
            drop(history);
            if let EventKind::MenuSelected(id) = &event.kind
                && ["checked", "radio-a", "radio-b"].contains(&id.as_str())
                && let Ok(trays) = collection(cx)
                && let Ok(handle) = trays.get("main")
                && let Ok(spec) = handle.specification()
                && let Some(mut menu) = spec.menu_description().cloned()
            {
                select_option(&mut menu, id);
                drop(handle.set_menu(menu));
            }
            if event.id == "main"
                && event.kind == EventKind::MenuSelected("open".into())
                && let Err(error) = rake::show_main_window(cx)
            {
                rake::error!(%error, "main window could not open");
            }
        })
        .quit_guard(move |_| {
            let blocked = guard.block_quit.get();
            async move {
                if blocked {
                    Err(rake::router::GuardReason(
                        "Quit is blocked by the workbench switch".into(),
                    ))
                } else {
                    Ok(())
                }
            }
        })
        .close_policy(rake::ClosePolicy::KeepRunning)
        .main_window(gpui::WindowOptions::default, move |runtime, _, cx| {
            content.mounts.set(content.mounts.get() + 1);
            Ok(cx
                .new(|_| Panel {
                    model: content.clone(),
                    runtime,
                })
                .into())
        });
    let outcome = Rc::new(RefCell::new(None));
    let app = if std::env::args().any(|arg| arg == "--probe") {
        let outcome = outcome.clone();
        app.background(move |runtime, cx| {
            let task = cx.spawn(async move |cx| {
                let result = probe(&runtime, &model, cx)
                    .await
                    .map_err(|error| error.to_string());
                let passed = result.is_ok();
                *outcome.borrow_mut() = Some(result);
                if passed {
                    println!("GPUI tray probe passed: native windows, focus/reopen, popup cycles, removal and guarded quit");
                    drop(runtime.quit(Default::default()));
                } else {
                    eprintln!("GPUI tray probe failed: {:?}", outcome.borrow());
                    cx.update(|cx| cx.quit());
                }
            });
            cx.set_global(ProbeTask(task));
        })
    } else {
        app
    };
    app.run()?;
    if std::env::args().any(|arg| arg == "--probe") {
        outcome
            .borrow_mut()
            .take()
            .ok_or("native tray probe did not finish")?
            .map_err(std::io::Error::other)?;
        println!(
            "GPUI tray probe passed: native windows, focus/reopen, popup switching, removal and guarded quit"
        );
    }
    Ok(())
}
struct PopupContent(String);
impl Render for PopupContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
            .size_full()
            .bg(gpui::rgb(0x14202b))
            .text_color(gpui::rgb(0xf1f5f9))
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child("Rake rich popup")
            .child(self.0.clone())
            .child("Escape or click outside to dismiss")
    }
}

struct ProbeTask(
    #[expect(dead_code, reason = "task field owns the native probe until app exit")] gpui::Task<()>,
);
impl gpui::Global for ProbeTask {}
async fn frame(cx: &mut gpui::AsyncApp) -> Result<(), Box<dyn std::error::Error>> {
    let (send, receive) = futures::channel::oneshot::channel();
    cx.update(|cx| {
        let handle = cx
            .windows()
            .into_iter()
            .next()
            .ok_or("probe has no window")?;
        handle.update(cx, |_, window, _| {
            window.on_next_frame(move |_, _| {
                let _ = send.send(());
            })
        })?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    receive.await?;
    Ok(())
}
async fn probe(
    runtime: &rake::runtime::RuntimeContext,
    model: &Model,
    cx: &mut gpui::AsyncApp,
) -> Result<(), Box<dyn std::error::Error>> {
    cx.update(|cx| flush(cx)).await?;
    let trays = runtime.trays()?;
    if trays.items()?.len() != 2 {
        return Err("expected two startup items".into());
    }
    cx.update(rake::show_main_window)?;
    frame(cx).await?;
    cx.update(rake::show_main_window)?;
    if model.mounts.get() != 1 {
        return Err("focus remounted main window".into());
    }
    cx.update(|cx| {
        for window in cx.windows() {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    });
    cx.update(rake::show_main_window)?;
    frame(cx).await?;
    if model.mounts.get() != 2 {
        return Err("main window did not recreate".into());
    }
    cx.update(|cx| toggle_popup("status", cx))?;
    frame(cx).await?;
    cx.update(|cx| dismiss_popup("status", cx))?;
    cx.update(|cx| toggle_popup("status", cx))?;
    frame(cx).await?;
    cx.update(|cx| dismiss_popup("status", cx))?;
    let transient = trays
        .create(Tray::new("probe-transient").icon(icon([100, 220, 150, 255])?))
        .await?;
    transient.remove().await?;
    model.block_quit.set(true);
    if !matches!(
        runtime.quit(Default::default()).wait().await?,
        rake::router::Outcome::Blocked(_)
    ) {
        return Err("guarded quit was not declined".into());
    }
    if trays.items()?.len() != 2 {
        return Err("declined quit removed native items".into());
    }
    model.block_quit.set(false);
    Ok(())
}

fn select_option(menu: &mut Menu, id: &str) {
    for item in &mut menu.items {
        match &mut item.kind {
            ItemKind::Check(state) if item.id == id => {
                *state = if *state == CheckState::On {
                    CheckState::Off
                } else {
                    CheckState::On
                }
            }
            ItemKind::Radio { selected, .. } if id.starts_with("radio-") => {
                *selected = item.id == id
            }
            ItemKind::Submenu(menu) => select_option(menu, id),
            _ => {}
        }
    }
}
