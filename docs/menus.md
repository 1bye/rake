# Application menus

Native menu items reference registered `ApplicationCommand`s by stable ID. The
menu bar, dock, in-app actions, agents and `rake ctl commands run` submit to the
same runtime command scheduler. Commands retain their argument decoding,
capability checks, cancellation tokens, operation history and result reporting.
Menu actions are distinct submissions, without transport retry keys.

Run the example:

```sh
RAKE_DEV_CONTROL=1 cargo run -p rake-app --example menus
# In another terminal, select the example's instance if several are running:
rake instances --json
rake ctl commands run example.toggle --args 'null' --json
```

Both the Options menu and the CLI call `example.toggle`. The command changes its
own checked state and enables/disables Perform action. The menu bar and dock
update together. Development control
is optional: menus work when it is disabled.

## Declare menus

Register handlers with `Commands::register` or `App::command`, then supply the
same registry to `App::commands`. Set each command's label and optional GPUI
shortcut through `Commands::set_presentation`. Without presentation metadata the
label defaults to the stable ID and there is no shortcut.

```rust
use rake::menus::{Menu, MenuItem, MenuRole, Menus};
use serde_json::json;

let menus = Menus::new([
    Menu::new("Application", [
        MenuItem::command("app.about", json!(null)),
        MenuItem::Separator,
        MenuItem::Role(MenuRole::Services),
        MenuItem::Role(MenuRole::Hide),
        MenuItem::Role(MenuRole::HideOthers),
        MenuItem::Role(MenuRole::ShowAll),
        MenuItem::Separator,
        MenuItem::Role(MenuRole::Quit),
    ]),
    Menu::new("File", [MenuItem::Submenu(Menu::new("Export", [
        MenuItem::command("document.export", json!({"format": "text"})),
    ]))]),
]);
// app.commands(commands).menus(menus).run()?;
```

`MenuItem::typed(command)` serializes arguments and gets the command's stable ID.
`MenuItem::command(id, arguments)` supports stored/dynamic menu specifications.
Unknown IDs, including those inside submenus or dock items, return
`CommandError::Unknown(id)` before `App::run` initializes the native host. Argument
decoding still happens when the action is submitted, just as with the CLI.

About is an ordinary registered command: the application owns its about dialog,
content and handler. Services and Hide roles use the OS behavior where supported.
Quit always uses the existing guarded quit action, including document/application
guards, decisions, cancellation and managed-work draining. Omitting `App::menus`
installs an Application menu containing guarded Quit. A custom menu replaces this
default; include the Quit role where desired. The quit shortcut remains available.

`Cut`, `Copy`, `Paste` and `SelectAll` preserve macOS native responder-chain
roles. `Undo` and `Redo` retain the pinned backend's action mapping; applications
should supply command items for their own undo/redo history. They do not invent application handlers for custom-drawn
editors. Such editors should register their editing operations as commands or use
their existing focused action handlers.

## Live state and keyboard shortcuts

Keep a clone of `Commands` and publish current application state with
`Commands::set_state(id, CommandState { enabled, checked, label })`. The default is
enabled, unchecked, with no label override. The registry is authoritative: a
disabled command is rejected from the CLI and agents as well as menus. Previously
admitted work continues; disabling is not cancellation.

Mounted hosts observe changes without polling. They rebuild native menus and
command shortcuts after state, presentation or menu-model changes. Updates before
the refresh task's first poll are retained. `Menus::replace(bar, dock, &commands)`
validates both surfaces before replacing either. Use the application's actual
registry for replacements; snapshots from another registry do not register commands
in the running application. `with_dock` configures initial dock items.

Shortcuts come from command presentation, rather than separate menu labels. They
are global GPUI bindings, and the native menu uses the same action and arguments
when displaying its key equivalent. Use platform-specific values (`cmd-s` on
macOS, `ctrl-s` elsewhere) or GPUI's supported modifier syntax. Changing the
presentation updates the binding; removing its last menu occurrence removes the
binding. Unrelated application bindings are retained. If an ID appears with
different arguments, its first occurrence in menu-bar order, then dock order,
gets the shortcut. Other occurrences remain clickable. Avoid assigning the same
shortcut to different commands or overriding the platform's Quit shortcut.

## Dock and context menus

`Menus::with_dock` and the dock argument of `Menus::replace` accept the same item
model, including submenus, separators and command references. On macOS GPUI hands
these to the native dock menu. Windows uses its supported taskbar integration;
Linux's pinned dock-menu backend is a no-op.

For context menus, `rake::menus::context_items(&items, &commands)` resolves the same
model to GPUI menu items, including current labels, checks, disabled state and
command actions. Present them using the application's existing popup UI. Resolve
each time the popup opens: an open context menu is a snapshot. The pinned GPUI
API has no portable arbitrary native context-menu presenter. The shared menu host
must be installed through `App::menus` (or the default managed host) for its command
actions to dispatch. An in-app popup can also call `MenuItem::run(&client)` and
observe the returned `Reply::Submitted` operation.

## Platform behavior

- **macOS:** native application and dock menus, Services, native editing roles,
  Hide actions, checks, disabled state and key equivalents use pinned
  `gpui-pre` 0.3.11 APIs.
- **Windows:** the backend retains application-menu models and supports dock/taskbar
  integration; Rake does not draw an additional menu bar. Host roles only have the
  behavior supplied by that backend.
- **Linux:** the backend retains application-menu models but does not implement a
  native menu bar or dock menu. Rake does not add a custom-drawn bar. Commands stay
  available through in-app UI and the runtime client.
- **Web:** declarations are validated, but native menu installation, dock menus and
  menu keyboard bindings are a no-op. Browsers own these surfaces and shortcuts.
  Expose commands through existing in-app controls; Rake does not inject a fallback
  bar or replace browser Quit behavior.

Menu tests exercise command/wire parity and state resolution without a display.
The `view-test-support` suite also dispatches installed Quit actions and verifies
live native-model and keybinding updates using the test platform. Those tests do
not establish real window-system behavior on every OS.
