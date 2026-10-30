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

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let json = arguments.iter().any(|argument| argument == "--json");

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
    if arguments.first().is_some_and(|command| command == "bundle")
        && distribution::handles(&arguments[1..])
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
