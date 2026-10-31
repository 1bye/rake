//! Host driver for standalone roots and no-window application runtimes.
//!
//! A managed root uses the same `Router<NoRoute>` scheduler as a routed
//! workspace. This module owns only the GPUI host task; the portable runtime
//! owns commands, application tasks, quit state and update handoff state.

use gpui::{
    AnyView, AppContext, Context, IntoElement, ParentElement, Render, Styled, Window, div, rgb,
};
#[cfg(feature = "updates")]
use gpui::{InteractiveElement, StatefulInteractiveElement};

use crate::control::{Commands, ControlConfig};
use crate::inspection::InspectElement;
#[cfg(not(feature = "devtools"))]
use crate::inspection::InspectRoot as _;
use crate::router::RouterConfig;
use crate::runtime::Runtime;
use crate::services::Services;

use super::ClosePolicy;

/// Configuration shared by windowed managed roots and background hosts.
#[derive(Clone)]
pub(super) struct ManagedConfig {
    pub(super) config: RouterConfig,
    pub(super) control: ControlConfig,
    pub(super) commands: Commands,
    pub(super) application_id: String,
    pub(super) close_policy: ClosePolicy,
    pub(super) quit_guards: Vec<crate::router::runtime::QuitGuard>,
}

pub(super) fn mount(
    content: super::Root,
    services: Services,
    config: ManagedConfig,
    window: &mut Window,
    cx: &mut gpui::App,
) -> AnyView {
    let runtime = prepare(&services, &config, window, cx);
    let context = runtime.context();
    let content = content(context, window, cx);

    cx.new(|cx| ManagedRoot::new(content, runtime, config, cx))
        .into()
}

pub(super) fn background(
    setup: super::Background,
    services: Services,
    config: ManagedConfig,
    cx: &mut gpui::App,
) {
    let runtime = prepare_background(&services, &config, cx);
    let context = runtime.context();
    setup(context, cx);

    let router = runtime.router.clone();
    let quit = cx.on_app_quit(move |_| {
        router.shutdown();
        async {}
    });
    let driver = spawn_background_driver(runtime, config, cx);
    cx.set_global(ManagedDriver {
        _driver: driver,
        _quit: quit,
    });
}

fn prepare(
    services: &Services,
    config: &ManagedConfig,
    window: &Window,
    cx: &mut gpui::App,
) -> Runtime {
    let runtime = Runtime::new(config.config.clone(), services.clone());
    runtime.router.install_quit_action(cx);
    configure(&runtime, config);

    if config.close_policy == ClosePolicy::KeepRunning {
        cx.set_quit_mode(gpui::QuitMode::Explicit);
    }

    // Both policies retain host visibility/unload observation. KeepRunning
    // detaches this window on close instead of submitting application quit.
    match config.close_policy {
        ClosePolicy::Quit => runtime.router.observe_host(window, cx),
        ClosePolicy::KeepRunning => runtime.router.observe_root_host(window, cx),
    }

    runtime
}

fn prepare_background(services: &Services, config: &ManagedConfig, cx: &mut gpui::App) -> Runtime {
    let runtime = Runtime::new(config.config.clone(), services.clone());
    runtime.router.install_quit_action(cx);
    configure(&runtime, config);
    cx.set_quit_mode(gpui::QuitMode::Explicit);
    runtime
}

fn configure(runtime: &Runtime, config: &ManagedConfig) {
    runtime
        .router
        .client()
        .set_commands(config.commands.clone());

    for guard in config.quit_guards.iter().cloned() {
        if let Err(error) = runtime.router.quit_guard(move |context| guard(context)) {
            crate::error!(%error, "managed quit guard could not be registered");
        }
    }

    #[cfg(feature = "devtools")]
    if config.control.inspection_enabled() {
        runtime.router.client().inspection().attach();
    }

    #[cfg(all(feature = "updates-sparkle", target_os = "macos"))]
    if let Err(error) = runtime.router.install_sparkle() {
        tracing::error!(%error, "native updater could not start");
    }
}

struct ManagedRoot {
    content: AnyView,
    router: crate::router::Router<crate::router::NoRoute>,
}

impl ManagedRoot {
    fn new(
        content: AnyView,
        runtime: Runtime,
        config: ManagedConfig,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = cx.weak_entity();
        let router = runtime.router.clone();
        let driver = spawn_root_driver(runtime, config, Some(view), cx);
        let quit_router = router.clone();
        let quit = cx.on_app_quit(move |_, _| {
            quit_router.shutdown();
            async {}
        });
        cx.set_global(ManagedDriver {
            _driver: driver,
            _quit: quit,
        });

        Self { content, router }
    }
}

impl Drop for ManagedRoot {
    fn drop(&mut self) {
        self.router.detach_host(crate::router::WindowId(1));
        #[cfg(feature = "devtools")]
        self.router
            .client()
            .inspection()
            .detach_window(crate::router::WindowId(1));
    }
}

impl Render for ManagedRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.router.snapshot();
        let inspection = crate::__rake_inspection!(
            self.router.client().inspection(),
            crate::inspection::Store::default()
        );

        if matches!(
            snapshot.quit.phase,
            crate::quit::QuitPhase::Stopping | crate::quit::QuitPhase::Ready
        ) {
            #[cfg(feature = "updates")]
            if let Some(error) = self.router.update_restart_error() {
                return div()
                    .size_full()
                    .bg(rgb(0x18232b))
                    .text_color(rgb(0xf0f4e8))
                    .p_4()
                    .child(format!(
                        "The update could not restart the app: {error}. The app finished shutting down. Restart manually to continue."
                    ))
                    .child(div().id("update-quit-manually").p_3().cursor_pointer().child("Quit")
                        .on_click(|_, _, cx| cx.quit()))
                    .inspect("rake.update.restart_error")
                    .root(inspection, snapshot.revision, crate::router::WindowId(1))
                    .into_any_element();
            }

            return div()
                .size_full()
                .bg(rgb(0x18232b))
                .text_color(rgb(0xf0f4e8))
                .p_4()
                .child(if snapshot.quit.phase == crate::quit::QuitPhase::Ready {
                    "Application closed. You can close this page."
                } else {
                    "Stopping managed work…"
                })
                .inspect("rake.quit")
                .root(inspection, snapshot.revision, crate::router::WindowId(1))
                .into_any_element();
        }

        if snapshot.reload.request.is_some() {
            let router = self.router.clone();

            if snapshot.reload.phase == crate::reload::ReloadPhase::Quiescing {
                window.on_next_frame(move |_, _| {
                    router.acknowledge_reload_ui(crate::router::WindowId(1))
                });
                cx.notify();
            }

            return div()
                .size_full()
                .bg(rgb(0x18232b))
                .text_color(rgb(0xf0f4e8))
                .p_4()
                .child("Preparing development reload…")
                .inspect("rake.reload")
                .root(inspection, snapshot.revision, crate::router::WindowId(1))
                .into_any_element();
        }

        crate::inspection::inspect(self.content.clone())
            .semantic_id("rake.root")
            .root(inspection, snapshot.revision, crate::router::WindowId(1))
            .into_any_element()
    }
}

struct ManagedDriver {
    _driver: Driver,
    _quit: gpui::Subscription,
}

impl gpui::Global for ManagedDriver {}

#[cfg(not(target_family = "wasm"))]
fn spawn_root_driver(
    runtime: Runtime,
    config: ManagedConfig,
    view: Option<gpui::WeakEntity<ManagedRoot>>,
    cx: &mut Context<ManagedRoot>,
) -> Driver {
    Driver {
        _task: cx.spawn(async move |entity, cx| {
            drive(runtime, config, view.or(Some(entity)), cx).await;
        }),
    }
}

#[cfg(target_family = "wasm")]
fn spawn_root_driver(
    runtime: Runtime,
    config: ManagedConfig,
    view: Option<gpui::WeakEntity<ManagedRoot>>,
    cx: &mut Context<ManagedRoot>,
) -> Driver {
    let mut context = cx.to_async();
    let (abort, registration) = futures::future::AbortHandle::new_pair();

    wasm_bindgen_futures::spawn_local(async move {
        let _ = futures::future::Abortable::new(
            drive(runtime, config, view, &mut context),
            registration,
        )
        .await;
    });

    Driver { abort }
}

#[cfg(not(target_family = "wasm"))]
fn spawn_background_driver(runtime: Runtime, config: ManagedConfig, cx: &mut gpui::App) -> Driver {
    Driver {
        _task: cx.spawn(async move |cx| {
            drive(runtime, config, None, cx).await;
        }),
    }
}

#[cfg(target_family = "wasm")]
fn spawn_background_driver(runtime: Runtime, config: ManagedConfig, cx: &mut gpui::App) -> Driver {
    let mut context = cx.to_async();
    let (abort, registration) = futures::future::AbortHandle::new_pair();

    wasm_bindgen_futures::spawn_local(async move {
        let _ = futures::future::Abortable::new(
            drive(runtime, config, None, &mut context),
            registration,
        )
        .await;
    });

    Driver { abort }
}

#[cfg(not(target_family = "wasm"))]
struct Driver {
    _task: gpui::Task<()>,
}

#[cfg(target_family = "wasm")]
struct Driver {
    abort: futures::future::AbortHandle,
}

#[cfg(target_family = "wasm")]
impl Drop for Driver {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

async fn drive(
    runtime: Runtime,
    config: ManagedConfig,
    view: Option<gpui::WeakEntity<ManagedRoot>>,
    cx: &mut gpui::AsyncApp,
) {
    // Keep the portable owner alive for the entire host driver lifetime. The
    // weak context handed to roots/pages must not be the owner of this state.
    let router = runtime.router.clone();
    let client = router.client();
    #[cfg(not(target_family = "wasm"))]
    let closing = client.clone();
    let application_id = config.application_id.clone();
    let control_config = config.control;

    let execute = async move {
        if let Err(error) = router.run().await {
            crate::error!(%error, "managed application driver stopped");
        }
    };

    let observed = runtime.router.clone();
    let observe = async move {
        let mut revision = u64::MAX;

        loop {
            #[cfg(all(feature = "updates", not(target_family = "wasm")))]
            let changed = Box::pin(observed.changed_with_updates(revision));
            #[cfg(all(
                not(feature = "updates"),
                feature = "native-lifecycle",
                target_os = "macos"
            ))]
            let changed = Box::pin(observed.changed_with_native_quit(revision));
            #[cfg(not(any(
                all(feature = "updates", not(target_family = "wasm")),
                all(
                    not(feature = "updates"),
                    feature = "native-lifecycle",
                    target_os = "macos"
                )
            )))]
            let changed = Box::pin(observed.changed(revision));

            #[cfg(feature = "devtools")]
            {
                let inspection = observed.client().inspection().listen();
                if let futures::future::Either::Left((next, _)) =
                    futures::future::select(changed, Box::pin(inspection)).await
                {
                    revision = next;
                }
            }
            #[cfg(not(feature = "devtools"))]
            {
                revision = changed.await;
            }

            #[cfg(not(target_family = "wasm"))]
            if observed.quit_status().phase == crate::quit::QuitPhase::Ready {
                closing.responses_idle().await;

                #[cfg(all(feature = "native-lifecycle", target_os = "macos"))]
                observed.prepare_native_exit(true);

                #[cfg(feature = "updates")]
                match observed.complete_update_restart() {
                    Ok(rake_updates::RestartDisposition::ProviderHandlesExit) => continue,
                    Ok(rake_updates::RestartDisposition::ExitHost) => {}
                    Err(error) => {
                        #[cfg(all(feature = "native-lifecycle", target_os = "macos"))]
                        {
                            observed.reply_native_quit(false);
                            observed.prepare_native_exit(true);
                        }
                        crate::error!(%error, "managed update handoff failed; manual restart required");
                        notify(view.as_ref(), cx).await;
                        break;
                    }
                }

                #[cfg(all(feature = "native-lifecycle", target_os = "macos"))]
                observed.reply_native_quit(true);

                cx.update(|cx| cx.quit());
                break;
            }

            #[cfg(target_family = "wasm")]
            if let Some(view) = &view
                && view.update(cx, |_, cx| cx.notify()).is_err()
            {
                break;
            }

            #[cfg(not(target_family = "wasm"))]
            notify(view.as_ref(), cx).await;
        }
    };

    let control = crate::control::run_host(control_config, client, application_id);
    let host = futures::future::select(Box::pin(execute), Box::pin(observe));
    futures::future::select(Box::pin(host), Box::pin(control)).await;
}

#[cfg(not(target_family = "wasm"))]
async fn notify(view: Option<&gpui::WeakEntity<ManagedRoot>>, cx: &mut gpui::AsyncApp) {
    if let Some(view) = view {
        let _ = view.update(cx, |_, cx| cx.notify());
    }
}

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
mod tests {
    use super::*;
    use crate::control::{ApplicationCommand, CommandDescriptor, ExecutionCapability, Reply};
    use crate::router::OperationError;
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[derive(Serialize, Deserialize)]
    struct ProbeCommand;

    impl ApplicationCommand for ProbeCommand {
        type Output = String;

        fn descriptor() -> CommandDescriptor {
            CommandDescriptor {
                id: "managed.probe".into(),
                arguments: json!({"type": "null"}),
                result: json!({"type": "string"}),
                capability: ExecutionCapability::Headless,
            }
        }
    }

    fn config(commands: Commands) -> ManagedConfig {
        ManagedConfig {
            config: RouterConfig::default(),
            control: ControlConfig::Disabled,
            commands,
            application_id: "managed.test".into(),
            close_policy: ClosePolicy::Quit,
            quit_guards: Vec::new(),
        }
    }

    #[test]
    fn managed_root_uses_one_runtime_for_commands_tasks_and_rendering() {
        let mut app = gpui::TestAppContext::single();
        let services = crate::App::new("managed.test").services().unwrap();
        let commands = Commands::new();
        let command_ran = Rc::new(Cell::new(false));
        let task_ran = Rc::new(Cell::new(false));
        let command_ran_in_handler = command_ran.clone();
        commands
            .register::<ProbeCommand, _, _>(move |_, _| {
                command_ran_in_handler.set(true);
                async { Ok("pong".into()) }
            })
            .unwrap();

        let (_, cx) = app.add_window_view(|_, cx| {
            let runtime = Runtime::new(RouterConfig::default(), services.clone());
            let managed_config = config(commands.clone());
            configure(&runtime, &managed_config);
            let context = runtime.context();

            let reply = context
                .client()
                .unwrap()
                .dispatch(ProbeCommand, "probe".into())
                .unwrap();
            assert!(matches!(reply, Reply::Submitted { .. }));
            let task_ran_in_task = task_ran.clone();
            context
                .tasks()
                .spawn("managed-probe", async move {
                    task_ran_in_task.set(true);
                    Ok(())
                })
                .unwrap();

            let content: AnyView = cx.new(|_| gpui::Empty).into();
            ManagedRoot::new(content, runtime, managed_config, cx)
        });

        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
        assert!(command_ran.get(), "managed command handler did not run");
        assert!(task_ran.get(), "managed application task did not run");
        cx.update(|window, _| window.remove_window());
        cx.run_until_parked();
    }

    #[cfg(feature = "devtools")]
    struct CapturedContent;

    #[cfg(feature = "devtools")]
    impl Render for CapturedContent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
                .w(gpui::px(120.))
                .h(gpui::px(40.))
                .inspect("managed.content")
        }
    }

    #[cfg(feature = "devtools")]
    #[test]
    fn managed_root_captures_content_without_route_identity() {
        let mut app = gpui::TestAppContext::single();
        let services = crate::App::new("managed.inspection").services().unwrap();
        let commands = Commands::new();
        let mut managed_config = config(commands);
        managed_config.control = ControlConfig::FromEnvironment;
        let inspection = Rc::new(RefCell::new(None));
        let inspection_in_root = inspection.clone();

        let (_, cx) = app.add_window_view(move |_, cx| {
            let runtime = Runtime::new(RouterConfig::default(), services.clone());
            runtime.router.client().inspection().attach();
            *inspection_in_root.borrow_mut() = Some(runtime.router.client().inspection());
            let content: AnyView = cx.new(|_| CapturedContent).into();
            ManagedRoot::new(content, runtime, managed_config, cx)
        });

        cx.update(|window, cx| window.draw(cx).clear(cx));
        let inspection = inspection.borrow().as_ref().unwrap().clone();
        let snapshot = inspection.snapshot();
        let content = snapshot
            .elements
            .iter()
            .find(|element| element.semantic_id.as_deref() == Some("managed.content"))
            .expect("managed content was not captured");
        assert_eq!(content.layout.unwrap().width, 120.);
        assert_eq!(content.layout.unwrap().height, 40.);
        assert!(
            snapshot
                .elements
                .iter()
                .all(|element| element.tab_id.is_none() && element.entry_id.is_none())
        );

        cx.update(|window, _| window.remove_window());
        cx.run_until_parked();
    }

    #[test]
    fn runtime_context_is_weak_after_owner_teardown() {
        let services = crate::App::new("managed.weak").services().unwrap();
        let runtime = Runtime::new(RouterConfig::default(), services);
        let context = runtime.context();

        assert!(context.client().is_ok());
        drop(runtime);
        assert!(matches!(
            context.client(),
            Err(OperationError::RuntimeShutdown)
        ));
    }
}
