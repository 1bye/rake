//! A command-backed native menu; run with RAKE_DEV_CONTROL=1 for CLI discovery.
#[cfg(feature = "gpui")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use rake::control::{CommandPresentation, CommandState, Commands};
    use rake::menus::{Menu, MenuItem, MenuRole, Menus};

    #[rake::data(schema)]
    #[derive(rake::Command)]
    #[command(id = "example.toggle", output = bool, capability = Headless)]
    struct Toggle;

    #[rake::data(schema)]
    #[derive(rake::Command)]
    #[command(id = "example.perform", capability = Headless)]
    struct Perform;

    #[rake::data(schema)]
    #[derive(rake::Command)]
    #[command(id = "example.about", output = String, capability = Headless)]
    struct About;

    let commands = Commands::new();

    commands.register::<Perform, _, _>(|_, _| async {
        rake::info!("Example command ran");

        Ok(())
    })?;
    commands.register::<About, _, _>(|_, _| async {
        let message = "Rake command menu example".to_owned();

        rake::info!(%message);

        Ok(message)
    })?;
    commands.set_presentation(
        "example.perform",
        CommandPresentation {
            label: "Perform action".into(),
            shortcut: None,
        },
    )?;
    commands.set_presentation(
        "example.about",
        CommandPresentation {
            label: "About menu example".into(),
            shortcut: None,
        },
    )?;
    commands.set_state(
        "example.perform",
        CommandState {
            enabled: false,
            ..Default::default()
        },
    )?;

    commands.register::<Toggle, _, _>(|_, context| async move {
        let commands = context
            .services
            .get::<Commands>()
            .map_err(|error| rake::control::CommandError::Failed(error.to_string()))?;

        let checked = !commands.presentation("example.toggle")?.1.checked;

        commands.set_state(
            "example.toggle",
            CommandState {
                checked,
                ..Default::default()
            },
        )?;

        commands.set_state(
            "example.perform",
            CommandState {
                enabled: checked,
                ..Default::default()
            },
        )?;

        Ok(checked)
    })?;
    commands.set_presentation(
        "example.toggle",
        CommandPresentation {
            label: "Toggle option".into(),
            shortcut: Some(
                if cfg!(target_os = "macos") {
                    "cmd-t"
                } else {
                    "ctrl-t"
                }
                .into(),
            ),
        },
    )?;
    let toggle = MenuItem::typed(Toggle)?;

    let menus = Menus::new([
        Menu::new(
            "Application",
            [
                MenuItem::typed(About)?,
                MenuItem::Separator,
                MenuItem::Role(MenuRole::Services),
                MenuItem::Separator,
                MenuItem::Role(MenuRole::Hide),
                MenuItem::Role(MenuRole::Quit),
            ],
        ),
        Menu::new("Options", [toggle.clone(), MenuItem::typed(Perform)?]),
    ])
    .with_dock([toggle]);

    rake::App::new("example.menus")
        .service(commands.clone())?
        .commands(commands)
        .menus(menus)
        .control(rake::control::ControlConfig::FromEnvironment)
        .root(|_, _, cx| {
            use rake::gpui::AppContext as _;

            cx.new(|_| rake::gpui::Empty).into()
        })
        .run()?;

    Ok(())
}

#[cfg(not(feature = "gpui"))]
fn main() {
    eprintln!("the menus example requires GPUI");
}
