//! Covers: installed menu actions, guarded quit, live host updates and keybindings.
//! Real: GPUI test app, menu host, router driver and command registry.
//! Doubles: GPUI platform. Requires: view-test-support, no real display.

use super::*;
use crate::control::{CommandPresentation, CommandState};
use crate::router::{GuardReason, NoRoute, Router, RouterConfig};
use futures::{executor::LocalPool, task::LocalSpawnExt};
use serde_json::json;
use std::cell::Cell;

#[crate::data(schema)]
#[derive(crate::Command)]
#[command(id = "menu.probe", capability = Headless)]
struct Probe;

#[test]
fn installed_quit_item_requests_guarded_quit_and_preserves_denied_runtime() {
    let app = gpui::TestAppContext::single();
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let invoked = Rc::new(Cell::new(false));
    let observed = invoked.clone();

    router
        .quit_guard(move |_| {
            observed.set(true);

            async { Err(GuardReason("busy".into())) }
        })
        .unwrap();
    let services = crate::App::new("menus.quit.test").services().unwrap();

    app.update(|cx| {
        router.install_quit_action(cx);
        install(router.client(), &services, cx).unwrap();
        let menus = cx.get_menus().unwrap();
        let gpui::OwnedMenuItem::Action { action, .. } = &menus[0].items[0] else {
            panic!("Quit action")
        };

        cx.dispatch_action(action.as_ref());
    });
    let mut pool = LocalPool::new();
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driven.run().await.unwrap();
        })
        .unwrap();
    pool.run_until_stalled();
    assert!(invoked.get());
    assert_eq!(router.quit_status().phase, crate::quit::QuitPhase::Blocked);
    assert!(!router.is_shutdown());
}

#[test]
fn live_host_refreshes_before_first_poll_and_replaces_only_its_command_bindings() {
    let app = gpui::TestAppContext::single();
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let commands = router.client().commands();

    commands
        .register::<Probe, _, _>(|_, _| async { Ok(()) })
        .unwrap();
    commands
        .set_presentation(
            "menu.probe",
            CommandPresentation {
                label: "Probe".into(),
                shortcut: Some("ctrl-p".into()),
            },
        )
        .unwrap();
    let menus = Menus::new([Menu::new(
        "App",
        [MenuItem::command("menu.probe", json!(null))],
    )]);
    let services = crate::App::new("menus.refresh.test")
        .menus(menus.clone())
        .services()
        .unwrap();

    app.update(|cx| {
        install(router.client(), &services, cx).unwrap();
        commands
            .set_state(
                "menu.probe",
                CommandState {
                    enabled: false,
                    checked: true,
                    label: Some("Busy".into()),
                },
            )
            .unwrap();
    });
    app.run_until_parked();
    app.update(|cx| {
        let bar = cx.get_menus().unwrap();

        assert!(matches!(&bar[0].items[0], gpui::OwnedMenuItem::Action { name, disabled: true, checked: true, .. } if name == "Busy"));
        let bindings = cx.key_bindings();
        let bindings = bindings.borrow();

        assert_eq!(bindings.bindings().filter(|binding| binding.action().as_any().is::<Invoke>()).count(), 1);
    });
    menus
        .replace(vec![Menu::new("Empty", [])], Vec::new(), &commands)
        .unwrap();
    app.run_until_parked();
    app.update(|cx| {
        assert_eq!(cx.get_menus().unwrap()[0].name.as_ref(), "Empty");
        assert_eq!(
            cx.key_bindings()
                .borrow()
                .bindings()
                .filter(|binding| binding.action().as_any().is::<Invoke>())
                .count(),
            0
        );
        assert!(
            cx.key_bindings()
                .borrow()
                .bindings()
                .any(|binding| binding.action().as_any().is::<crate::quit::RequestQuit>())
        );
    });
}
