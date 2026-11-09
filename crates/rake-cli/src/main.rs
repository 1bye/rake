//! Terminal adapter for the shared development control protocol.

mod agent;
#[cfg(unix)]
mod cdp;
mod project_build;
#[cfg(unix)]
mod source;
#[cfg(unix)]
mod terminal;

mod distribution;
mod format;
mod lint;
mod native_commands;
mod release;
mod scaffold;
mod update_key;
mod view_source;

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();

    if arguments
        .first()
        .is_some_and(|argument| argument == "--version")
    {
        println!("rake {}", env!("CARGO_PKG_VERSION"));

        return;
    }

    let json = arguments.iter().any(|argument| argument == "--json");

    if arguments
        .first()
        .is_some_and(|command| matches!(command.as_str(), "format" | "fmt"))
    {
        match format::run(&arguments[1..]) {
            Ok(false) => return,
            Ok(true) => std::process::exit(1),
            Err(error) => {
                eprintln!("rake: {error}");
                std::process::exit(1);
            }
        }
    }

    if arguments.first().is_some_and(|command| command == "lint") {
        match lint::run(&arguments[1..]) {
            Ok(false) => return,
            Ok(true) => std::process::exit(1),
            Err(error) => {
                if json {
                    eprintln!(
                        "{}",
                        serde_json::json!({"error":{"message":error.to_string()}})
                    );
                } else {
                    eprintln!("rake: {error}");
                }

                std::process::exit(1);
            }
        }
    }

    if arguments.first().is_some_and(|command| {
        matches!(
            command.as_str(),
            "release" | "__release-worker" | "update-key" | "new"
        )
    }) {
        let result = if arguments[0] == "new" {
            scaffold::run(&arguments[1..])
        } else if arguments[0] == "update-key" {
            update_key::run(&arguments[1..])
        } else if arguments[0] == "release" {
            release::run(&arguments[1..])
        } else {
            release::worker(&arguments[1..])
        };

        if let Err(error) = result {
            if json {
                eprintln!(
                    "{}",
                    serde_json::json!({"error":{"message":error.to_string()}})
                );
            } else {
                eprintln!("rake: {error}");
            }

            std::process::exit(1);
        }

        return;
    }

    if arguments
        .first()
        .is_some_and(|command| command == "__distribution-worker")
    {
        let result = arguments
            .get(1)
            .ok_or("worker request missing")
            .map_err(Into::into)
            .and_then(|path| {
                distribution::worker(
                    std::path::Path::new(path),
                    std::path::Path::new(arguments.get(2).ok_or("worker workspace missing")?),
                )
            });

        if let Err(error) = result {
            eprintln!("rake worker: {error}");
            std::process::exit(1);
        }

        return;
    }

    if arguments.first().is_some_and(|command| {
        command == "__native-build" || native_commands::handles(command, &arguments[1..])
    }) {
        let result = if arguments[0] == "__native-build" {
            arguments
                .get(1)
                .ok_or("native build result path missing")
                .map_err(Into::into)
                .and_then(|output| {
                    native_commands::build_worker(std::path::Path::new(output), &arguments[2..])
                })
        } else {
            native_commands::run(&arguments[0], &arguments[1..])
        };

        if let Err(error) = result {
            if json {
                eprintln!(
                    "{}",
                    serde_json::json!({"error":{"message":error.to_string()}})
                );
            } else {
                eprintln!("rake: {error}");
            }

            std::process::exit(1);
        }

        return;
    }

    if arguments
        .first()
        .is_some_and(|command| matches!(command.as_str(), "doctor" | "build" | "dev"))
    {
        std::process::exit(project_build::run(&arguments[0], &arguments[1..]));
    }

    let web_bundle = arguments.first().is_some_and(|command| command == "bundle")
        && project_build::web_target(&arguments[1..]);

    if web_bundle
        && (arguments.iter().any(|arg| arg == "--config")
            || (!arguments.iter().any(|arg| arg == "--manifest-path")
                && std::path::Path::new("Bundle.toml").exists()))
    {
        let message = "standalone Web bundling is not supported yet (RFC step 6); use explicit --manifest-path for the existing Web adapter";

        if json {
            eprintln!("{}", serde_json::json!({"error":{"message":message}}));
        } else {
            eprintln!("rake: {message}");
        }

        std::process::exit(1);
    }

    if web_bundle {
        std::process::exit(project_build::run("bundle", &arguments[1..]));
    }

    if arguments.first().is_some_and(|command| command == "bundle") && !web_bundle {
        match distribution::run(&arguments[1..]) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                if json {
                    eprintln!(
                        "{}",
                        serde_json::json!({"error": {"message": error.to_string()}})
                    );
                } else {
                    eprintln!("rake: {error}");
                }

                std::process::exit(1);
            }
        }
    }

    if arguments.first().is_some_and(|command| command == "agent") {
        if let Err(error) = agent::run(arguments) {
            if json {
                eprintln!(
                    "{}",
                    serde_json::json!({"error":{"message":error.to_string()}})
                );
            } else {
                eprintln!("rake: {error}");
            }

            std::process::exit(1);
        }

        return;
    }

    #[cfg(unix)]
    let result = if arguments.first().is_some_and(|command| command == "cdp") {
        cdp::run(arguments)
    } else if arguments.first().is_some_and(|c| c == "inspect")
        && arguments.get(1).is_some_and(|c| c == "pick")
    {
        terminal::run(arguments)
    } else if arguments.first().is_some_and(|c| c == "inspect") {
        cdp::run(arguments)
    } else {
        terminal::run(arguments)
    };

    #[cfg(not(unix))]
    let result: Result<(), Box<dyn std::error::Error>> =
        Err("native local transport currently requires Unix".into());

    if let Err(error) = result {
        if json {
            eprintln!(
                "{}",
                serde_json::json!({"error": {"message": error.to_string()}})
            );
        } else {
            eprintln!("rake: {error}");
        }

        std::process::exit(1);
    }
}
