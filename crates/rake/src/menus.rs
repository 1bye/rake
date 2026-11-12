//! Command-backed application, dock and context menu models.
//!
//! Native hosts install [`Menus`] supplied with [`crate::App::menus`]. Web validates
//! the same model but does not install a browser menu bar or keyboard shortcuts.

use crate::control::{CommandError, Commands, Reply, RuntimeClient};
use crate::router::Route;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{cell::RefCell, rc::Rc};

/// A named menu, also usable as a nested submenu.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    /// Visible menu title.
    pub name: String,
    /// Ordered command, role, separator and submenu items.
    pub items: Vec<MenuItem>,
}

impl Menu {
    /// Creates a named menu from ordered items.
    pub fn new(name: impl Into<String>, items: impl IntoIterator<Item = MenuItem>) -> Self {
        Self {
            name: name.into(),
            items: items.into_iter().collect(),
        }
    }
}

/// Standard host actions. About is an application command, using its own stable ID.
/// Edit roles preserve macOS responder-chain behavior for system text controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[non_exhaustive]
pub enum MenuRole {
    /// Requests Rake's guarded quit, never immediate host termination.
    Quit,
    /// The macOS Services submenu.
    Services,
    /// Hides this application on supported native hosts.
    Hide,
    /// Hides other applications on supported native hosts.
    HideOthers,
    /// Unhides other applications on supported native hosts.
    ShowAll,
    /// Standard Cut responder action.
    Cut,
    /// Standard Copy responder action.
    Copy,
    /// Standard Paste responder action.
    Paste,
    /// Standard Select All responder action.
    SelectAll,
    /// Standard Undo responder action.
    Undo,
    /// Standard Redo responder action.
    Redo,
}

/// One item shared by the menu bar, dock and context menus.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MenuItem {
    /// A registered command with serialized arguments; presentation comes from its registry.
    Command {
        /// Stable `ApplicationCommand` ID.
        id: String,
        /// Arguments decoded by the registered command, exactly as CLI arguments are.
        arguments: Value,
    },
    /// An ordered nested menu.
    Submenu(Menu),
    /// A visual separator.
    Separator,
    /// A standard host action.
    Role(MenuRole),
}

impl MenuItem {
    /// References a registered command by stable ID and JSON arguments.
    pub fn command(id: impl Into<String>, arguments: Value) -> Self {
        Self::Command {
            id: id.into(),
            arguments,
        }
    }

    /// Serializes typed arguments while retaining the command's stable ID.
    /// # Errors
    /// Returns `Arguments` if serialization fails. Registration is validated at startup.
    pub fn typed<C: crate::control::ApplicationCommand>(command: C) -> Result<Self, CommandError> {
        Ok(Self::command(
            C::id(),
            serde_json::to_value(command)
                .map_err(|error| CommandError::Arguments(error.to_string()))?,
        ))
    }

    /// Runs a command item through the runtime scheduler, returning its observable operation.
    /// # Errors
    /// Rejects non-command items, unknown/disabled commands, invalid arguments and stopped runtimes.
    pub fn run<R: Route>(
        &self,
        client: &RuntimeClient<R>,
    ) -> Result<Reply, crate::control::ControlError> {
        match self {
            Self::Command { id, arguments } => {
                client.dispatch_local_command(id.clone(), arguments.clone())
            }
            _ => Err(
                CommandError::Arguments("menu item is not an application command".into()).into(),
            ),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Model {
    bar: Vec<Menu>,
    dock: Vec<MenuItem>,
}

/// Shared live menu configuration. Clones update the same mounted host.
///
/// ```
/// use rake::menus::{Menu, MenuItem, MenuRole, Menus};
/// let menus = Menus::new([Menu::new("Application", [MenuItem::Role(MenuRole::Quit)])]);
/// ```
#[derive(Clone, Default)]
pub struct Menus {
    model: Rc<RefCell<Model>>,
    changed: Rc<event_listener::Event>,
}

impl Menus {
    /// Creates a menu bar; dock items initially remain empty.
    pub fn new(bar: impl IntoIterator<Item = Menu>) -> Self {
        Self {
            model: Rc::new(RefCell::new(Model {
                bar: bar.into_iter().collect(),
                dock: Vec::new(),
            })),
            changed: Rc::default(),
        }
    }

    /// Replaces both surfaces atomically after validating all command references.
    /// Keep a clone of the `Commands` passed to the application for updates.
    /// # Errors
    /// Returns `Unknown` without changing either surface if any stable ID is unregistered.
    pub fn replace(
        &self,
        bar: Vec<Menu>,
        dock: Vec<MenuItem>,
        commands: &Commands,
    ) -> Result<(), CommandError> {
        let model = Model { bar, dock };

        validate_model(&model, commands)?;
        *self.model.borrow_mut() = model;
        self.changed.notify(usize::MAX);

        Ok(())
    }

    /// Sets initial dock items. All references are validated at application startup.
    pub fn with_dock(self, items: impl IntoIterator<Item = MenuItem>) -> Self {
        self.model.borrow_mut().dock = items.into_iter().collect();

        self
    }

    /// Validates every nested command reference against the supplied registry.
    /// # Errors
    /// Returns `Unknown` with the offending stable ID.
    pub fn validate(&self, commands: &Commands) -> Result<(), CommandError> {
        validate_model(&self.model.borrow(), commands)
    }
}

fn validate_items(items: &[MenuItem], commands: &Commands) -> Result<(), CommandError> {
    for item in items {
        match item {
            MenuItem::Command { id, .. } => {
                commands.presentation(id)?;
            }
            MenuItem::Submenu(menu) => validate_items(&menu.items, commands)?,
            MenuItem::Separator | MenuItem::Role(_) => {}
        }
    }

    Ok(())
}

fn validate_model(model: &Model, commands: &Commands) -> Result<(), CommandError> {
    for menu in &model.bar {
        validate_items(&menu.items, commands)?;
    }

    validate_items(&model.dock, commands)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, gpui::Action)]
#[action(namespace = rake_menus)]
struct Invoke {
    id: String,
    arguments: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, gpui::Action)]
#[action(namespace = rake_menus)]
struct Standard {
    role: MenuRole,
}

/// Resolves context menu items using the same current command state and actions.
/// The caller presents these items with its existing native or in-app popup UI.
/// Re-resolve when opening the popup; an already open popup is a snapshot.
/// # Errors
/// Returns `Unknown` for any unregistered command, including nested items.
pub fn context_items(
    items: &[MenuItem],
    commands: &Commands,
) -> Result<Vec<gpui::MenuItem>, CommandError> {
    validate_items(items, commands)?;

    items
        .iter()
        .map(|item| resolve_item(item, commands))
        .collect()
}

fn resolve_menu(menu: &Menu, commands: &Commands) -> Result<gpui::Menu, CommandError> {
    Ok(gpui::Menu::new(menu.name.clone()).items(context_items(&menu.items, commands)?))
}

fn resolve_item(item: &MenuItem, commands: &Commands) -> Result<gpui::MenuItem, CommandError> {
    Ok(match item {
        MenuItem::Separator => gpui::MenuItem::separator(),
        MenuItem::Submenu(menu) => gpui::MenuItem::submenu(resolve_menu(menu, commands)?),
        MenuItem::Role(role) => resolve_role(*role),
        MenuItem::Command { id, arguments } => {
            let (presentation, state) = commands.presentation(id)?;

            gpui::MenuItem::action(
                state.label.unwrap_or(presentation.label),
                Invoke {
                    id: id.clone(),
                    arguments: arguments.clone(),
                },
            )
            .checked(state.checked)
            .disabled(!state.enabled)
        }
    })
}

fn resolve_role(role: MenuRole) -> gpui::MenuItem {
    use MenuRole::*;
    let (label, os_action) = match role {
        Quit => return gpui::MenuItem::action("Quit", crate::quit::RequestQuit),
        Services => return gpui::MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
        Hide => ("Hide", None),
        HideOthers => ("Hide Others", None),
        ShowAll => ("Show All", None),
        Cut => ("Cut", Some(gpui::OsAction::Cut)),
        Copy => ("Copy", Some(gpui::OsAction::Copy)),
        Paste => ("Paste", Some(gpui::OsAction::Paste)),
        SelectAll => ("Select All", Some(gpui::OsAction::SelectAll)),
        Undo => ("Undo", Some(gpui::OsAction::Undo)),
        Redo => ("Redo", Some(gpui::OsAction::Redo)),
    };

    match os_action {
        Some(os_action) => gpui::MenuItem::os_action(label, Standard { role }, os_action),
        None => gpui::MenuItem::action(label, Standard { role }),
    }
}

struct Host {
    _refresh: gpui::Task<()>,
}

impl gpui::Global for Host {}

pub(crate) fn install<R: Route>(
    client: RuntimeClient<R>,
    services: &crate::Services,
    cx: &mut gpui::App,
) -> Result<(), CommandError> {
    let menus = services
        .get::<Menus>()
        .map(|menus| (*menus).clone())
        .unwrap_or_else(|_| default_menus());
    let commands = client.commands();

    menus.validate(&commands)?;

    // Browsers own their menu bar and shortcuts. Application commands remain in-app.
    if cfg!(target_family = "wasm") {
        return Ok(());
    }

    cx.on_action(move |action: &Invoke, _| {
        let item = MenuItem::command(action.id.clone(), action.arguments.clone());

        if let Err(error) = item.run(&client) {
            tracing::error!(%error, command = %action.id, "menu command rejected");
        }
    });
    cx.on_action(|action: &Standard, cx| match action.role {
        MenuRole::Hide => cx.hide(),
        MenuRole::HideOthers => cx.hide_other_apps(),
        MenuRole::ShowAll => cx.unhide_other_apps(),
        _ => cx.propagate(),
    });

    cx.bind_keys([gpui::KeyBinding::new(
        if cfg!(target_os = "macos") {
            "cmd-q"
        } else {
            "ctrl-q"
        },
        crate::quit::RequestQuit,
        None,
    )]);
    let mut model_changed = menus.changed.listen();
    let mut state_changed = commands.listen();
    refresh(&menus, &commands, cx)?;
    let refresh_task = cx.spawn(async move |cx| {
        loop {
            futures::future::select(Box::pin(model_changed), Box::pin(state_changed)).await;
            model_changed = menus.changed.listen();
            state_changed = commands.listen();
            cx.update(|cx| {
                if let Err(error) = refresh(&menus, &commands, cx) {
                    tracing::error!(%error, "menu refresh rejected");
                }
            });
        }
    });

    cx.set_global(Host {
        _refresh: refresh_task,
    });

    Ok(())
}

fn bind_items(
    items: &[MenuItem],
    commands: &Commands,
    bound: &mut std::collections::BTreeSet<String>,
    cx: &gpui::App,
    bindings: &mut Vec<gpui::KeyBinding>,
) -> Result<(), CommandError> {
    for item in items {
        match item {
            MenuItem::Command { id, arguments } if bound.insert(id.clone()) => {
                if let Some(shortcut) = commands.presentation(id)?.0.shortcut {
                    bindings.push(
                        gpui::KeyBinding::load(
                            &shortcut,
                            Box::new(Invoke {
                                id: id.clone(),
                                arguments: arguments.clone(),
                            }),
                            None,
                            false,
                            None,
                            cx,
                        )
                        .map_err(|error| {
                            CommandError::Arguments(format!("invalid shortcut for {id}: {error}"))
                        })?,
                    );
                }
            }
            MenuItem::Submenu(menu) => bind_items(&menu.items, commands, bound, cx, bindings)?,
            _ => {}
        }
    }

    Ok(())
}

fn refresh(menus: &Menus, commands: &Commands, cx: &mut gpui::App) -> Result<(), CommandError> {
    let model = menus.model.borrow();
    // Replace only this host's bindings, retaining unrelated application bindings.
    let retained = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|binding| !binding.action().as_any().is::<Invoke>())
        .cloned()
        .collect::<Vec<_>>();
    let mut bound = std::collections::BTreeSet::new();

    let mut bindings = Vec::new();

    for menu in &model.bar {
        bind_items(&menu.items, commands, &mut bound, cx, &mut bindings)?;
    }

    bind_items(&model.dock, commands, &mut bound, cx, &mut bindings)?;
    cx.clear_key_bindings();
    cx.bind_keys(retained);
    cx.bind_keys(bindings);
    let bar = model
        .bar
        .iter()
        .map(|menu| resolve_menu(menu, commands))
        .collect::<Result<Vec<_>, _>>()?;
    let dock = context_items(&model.dock, commands)?;

    cx.set_menus(bar);
    cx.set_dock_menu(dock);

    Ok(())
}

fn default_menus() -> Menus {
    Menus::new([Menu::new("Application", [MenuItem::Role(MenuRole::Quit)])])
}

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
mod tests;
