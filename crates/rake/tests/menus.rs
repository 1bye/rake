//! Covers: command menus, state refresh, validation and shared wire dispatch.
//! Real: registry, menu resolver, runtime scheduler and operation results.
//! Doubles: none. Requires: no GPU, display or network.
#![cfg(feature = "gpui")]

use futures::{executor::LocalPool, task::LocalSpawnExt};

use rake::control::{
    Command, CommandError, CommandPresentation, CommandState, Message, PROTOCOL_VERSION, Reply,
    Request,
};

use rake::menus::{Menu, MenuItem, MenuRole, Menus, context_items};
use rake::router::{NoRoute, Router, RouterConfig};
use serde_json::json;
use std::{cell::Cell, rc::Rc};

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "counter.add", output = u64, capability = Headless)]
struct Add {
    amount: u64,
}

#[test]
fn menu_and_wire_run_the_same_handler_and_reject_disabled_commands() {
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let client = router.client();
    let total = Rc::new(Cell::new(0));
    let changed = total.clone();

    client
        .commands()
        .register::<Add, _, _>(move |args, _| {
            let total = changed.clone();

            async move {
                total.set(total.get() + args.amount);

                Ok(total.get())
            }
        })
        .unwrap();
    let item = MenuItem::typed(Add { amount: 3 }).unwrap();
    let menu_reply = item.run(&client).unwrap();

    let wire_reply = client
        .request(Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: router.runtime_id(),
            key: Some("wire-add".into()),
            message: Message::Command(Command::Run {
                id: "counter.add".into(),
                arguments: json!({"amount": 3}),
            }),
        })
        .unwrap();

    assert!(matches!(menu_reply, Reply::Submitted { .. }));
    assert!(matches!(wire_reply, Reply::Submitted { .. }));
    let mut pool = LocalPool::new();
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driven.run().await.unwrap();
        })
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(total.get(), 6);

    client
        .commands()
        .set_state(
            "counter.add",
            CommandState {
                enabled: false,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(item.run(&client).is_err());
    assert!(client.execute(Add { amount: 3 }).is_err());
    pool.run_until_stalled();
    assert_eq!(total.get(), 6);
}

#[test]
fn state_changes_refresh_labels_checks_and_availability_in_nested_context_menus() {
    let commands = rake::control::Commands::new();

    commands
        .register::<Add, _, _>(|args, _| async move { Ok(args.amount) })
        .unwrap();
    commands
        .set_presentation(
            "counter.add",
            CommandPresentation {
                label: "Add".into(),
                shortcut: Some("ctrl-a".into()),
            },
        )
        .unwrap();
    let items = [MenuItem::Submenu(Menu::new(
        "Counter",
        [
            MenuItem::command("counter.add", json!({"amount": 1})),
            MenuItem::Separator,
        ],
    ))];

    let before = context_items(&items, &commands).unwrap();

    let rake::gpui::MenuItem::Submenu(menu) = &before[0] else {
        panic!("submenu")
    };

    assert!(!menu.items[0].is_disabled());
    assert!(!menu.items[0].is_checked());
    commands
        .set_state(
            "counter.add",
            CommandState {
                enabled: false,
                checked: true,
                label: Some("Added".into()),
            },
        )
        .unwrap();
    let after = context_items(&items, &commands).unwrap();

    let rake::gpui::MenuItem::Submenu(menu) = &after[0] else {
        panic!("submenu")
    };

    assert!(menu.items[0].is_disabled());
    assert!(menu.items[0].is_checked());
    assert!(
        matches!(&menu.items[0], rake::gpui::MenuItem::Action { name, .. } if name.as_ref() == "Added")
    );
}

#[test]
fn invalid_replacement_preserves_existing_menus_and_startup_reports_unknown_id() {
    let commands = rake::control::Commands::new();
    let menus = Menus::new([Menu::new("Application", [MenuItem::Role(MenuRole::Quit)])]);

    let unknown = MenuItem::Submenu(Menu::new(
        "Nested",
        [MenuItem::command("missing.command", json!(null))],
    ));

    assert!(
        matches!(menus.replace(Vec::new(), vec![unknown.clone()], &commands), Err(CommandError::Unknown(id)) if id == "missing.command")
    );
    menus.validate(&commands).unwrap();
    let result = rake::App::new("menus.test")
        .menus(Menus::new([Menu::new("App", [unknown])]))
        .run();

    assert!(
        matches!(result, Err(rake::AppError::Command(CommandError::Unknown(id))) if id == "missing.command")
    );
}

#[test]
fn invalid_shortcut_does_not_replace_command_presentation() {
    let commands = rake::control::Commands::new();

    commands
        .register::<Add, _, _>(|args, _| async move { Ok(args.amount) })
        .unwrap();
    let original = commands.presentation("counter.add").unwrap();

    let result = commands.set_presentation(
        "counter.add",
        CommandPresentation {
            label: "Broken".into(),
            shortcut: Some("invalidmodifier-a".into()),
        },
    );

    assert!(matches!(result, Err(CommandError::Arguments(_))));
    assert_eq!(commands.presentation("counter.add").unwrap(), original);
}
