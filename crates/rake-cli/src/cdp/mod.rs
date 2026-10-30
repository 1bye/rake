//! Loopback CDP adapter; runtime access always goes through the shared protocol.

mod accessibility;
mod causal;
mod console;
mod dom;
mod editing;
mod evaluate;
mod frames;
mod memory;
mod network;
mod server;
mod session;
mod sources;
mod storage;
mod styles;
mod trace;
mod wire;

use rake_client::local::{self, Instance};
use rake_protocol::control::{Query, Reply};
use std::path::{Path, PathBuf};
use std::time::Duration;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const TIMEOUT: Duration = Duration::from_secs(2);
const HELP: &str = "rake cdp [--session PATH] [--instance UUID|APPLICATION_ID] [--port PORT] [--no-open] [--json] [--export-trace FILE]\nDefault port: OS-assigned. Paste the printed devtools:// URL into Chrome, or use chrome://inspect.";

struct Options {
    directory: PathBuf,
    selector: Option<String>,
    port: u16,
    open: bool,
    json: bool,
    process_output: Option<PathBuf>,
    export_trace: Option<PathBuf>,
}

impl Options {
    fn parse(mut args: Vec<String>) -> Result<Self> {
        args.remove(0);
        let mut options = Self {
            directory: local::directory(),
            selector: None,
            port: 0,
            open: true,
            json: false,
            process_output: None,
            export_trace: None,
        };

        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--export-trace" => {
                    options.export_trace = Some(PathBuf::from(
                        args.next().ok_or("--export-trace requires a file")?,
                    ))
                }
                "--session" => {
                    let session = PathBuf::from(args.next().ok_or("--session requires a path")?);

                    options.directory = session.join("control");
                    options.process_output = Some(session.join("output.json"));
                }
                "--instance" => {
                    options.selector = Some(args.next().ok_or("--instance requires a selector")?)
                }
                "--port" => options.port = args.next().ok_or("--port requires a port")?.parse()?,
                "--no-open" => options.open = false,
                "--json" => options.json = true,
                _ => return Err(format!("unexpected CDP argument: {arg}\n{HELP}").into()),
            }
        }

        Ok(options)
    }
}

fn discover(directory: &Path, selector: Option<&str>) -> Result<Vec<Instance>> {
    Ok(local::discover(directory)?.into_iter().filter(|instance| {
        selector.is_none_or(|s| s == instance.application_id || s == instance.info.runtime_id.to_string())
            && matches!(crate::terminal::query(instance, Query::Info {}, TIMEOUT), Ok(Reply::Info(info)) if info.runtime_id == instance.info.runtime_id)
    }).collect())
}

pub(crate) fn run(args: Vec<String>) -> Result<()> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{HELP}");

        return Ok(());
    }

    let options = Options::parse(args)?;

    if let Some(path) = &options.export_trace {
        return trace::export(&options, path);
    }

    server::serve(options)
}
