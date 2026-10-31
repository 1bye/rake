//! Terminal adapter for the shared development control protocol.

#[cfg(unix)]
mod agent;
#[cfg(unix)]
mod cdp;
#[cfg(unix)]
mod project;
#[cfg(unix)]
mod source;
#[cfg(unix)]
mod terminal;

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let json = arguments.iter().any(|argument| argument == "--json");

    #[cfg(unix)]
    let result = if arguments.first().is_some_and(|command| command == "agent") {
        agent::run(arguments)
    } else if arguments.first().is_some_and(|command| command == "cdp") {
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
