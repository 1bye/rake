//! Terminal adapter for the shared development control protocol.

#[cfg(unix)]
mod cdp;
#[cfg(unix)]
mod project;
#[cfg(unix)]
mod source;
#[cfg(unix)]
mod terminal;

mod distribution;
mod native_commands;
mod release;
mod update_key;

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let json = arguments.iter().any(|argument| argument == "--json");

    if arguments.first().is_some_and(|command| {
        matches!(
            command.as_str(),
            "release" | "__release-worker" | "update-key"
        )
    }) {
        let result = if arguments[0] == "update-key" {
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
    if arguments.first().is_some_and(|command| command == "bundle")
        && !arguments
            .windows(2)
            .any(|pair| pair[0] == "--target" && pair[1] == "web")
    {
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

    #[cfg(unix)]
    let result = if arguments.first().is_some_and(|command| command == "cdp") {
        cdp::run(arguments)
    } else if arguments.first().is_some_and(|c| c == "inspect")
        && arguments.get(1).is_some_and(|c| c == "pick")
    {
        terminal::run(arguments)
    } else if arguments.first().is_some_and(|command| {
        matches!(
            command.as_str(),
            "dev" | "build" | "bundle" | "doctor" | "inspect"
        )
    }) {
        project::run(arguments)
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
