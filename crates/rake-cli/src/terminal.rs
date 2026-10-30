//! Converts terminal arguments to the same owned commands/queries as Rust clients.

use rake_client::local::{self, Instance};
use rake_protocol::control::{Command, Message, PROTOCOL_VERSION, Query, Reply, Request};

use rake_protocol::router::{
    Activation, OpenOptions, OpenTarget, OperationState, RouteAddress, TabMode,
};

use serde_json::json;
use std::error::Error;
use std::str::FromStr;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const HELP: &str = "rake inspect [--elements --target desktop|web] [--session PATH] [--instance UUID|application-id] [--no-open]
rake cdp [--session PATH] [--instance UUID|application-id] [--port PORT] [--no-open] [--json] [--export-trace FILE]
rake highlight ID|none [--generation CAPTURE] [--instance UUID|application-id]
rake inspect pick [--instance UUID|application-id]
rake instances [--json]
rake ctl [--instance UUID|application-id] [--json] [--timeout SECONDS]
  inspect tree | inspect element ELEMENT_ID | inspect pick | inspect mode on|off [--lease-ms MS]
  inspect focus|scroll ELEMENT_ID [--generation CAPTURE] | inspect source ELEMENT_ID [--style INDEX|--handler INDEX]
  info | schema | snapshot | events [--follow]
  quit [--jobs wait|cancel] | quit status
  reload status | reload checkpoint APPLICATION_ID
  reload prepare ID [--mechanism native_restart|web_page_reload|native_code_replacement]
  reload apply|abort ID
  routes list
  router open|reveal ROUTE [--param key=value] [--target new-tab|current-tab|new-window]
      [--window WINDOW] [--scope window|application] [--mode foreground|live] [--preserve-selection]
  windows list|get WINDOW|close WINDOW
  tabs list|select|close|back|forward TAB
  tabs mode TAB foreground|live
  tabs navigate|replace TAB ROUTE [--param key=value]
  commands list | commands run ID --args JSON
  tasks list [--owner application|tab-N|entry-N] | tasks cancel TASK
  operations list|get|watch|cancel OP
  decisions list | decisions respond DECISION --choice CHOICE
  logs list|follow | requests list | traces export [--after SEQUENCE]
  request JSON_MESSAGE
Quit reports submission; inspect decisions and quit status while the host is still running.
Mutations: [--no-wait] [--request-key KEY]. Retry the same payload and key after an uncertain response.
Raw messages use the shared {\"type\":\"query\"|\"command\",\"value\":{\"type\":...}} schema.
Development: RAKE_DEV_CONTROL=1 <native-app>; Web: scripts/serve-web.py --control.
Build/watch: rake dev|build --target desktop|web --package PACKAGE.
Package: rake bundle --target desktop|web --package PACKAGE [--debug] [--json].
Web deployment: --base-path /notes/ --routing path|hash.
macOS signing: --sign-identity IDENTITY [--entitlements FILE]; default ad-hoc.
Prerequisites: rake doctor. Graphical devtools: rake inspect, or the Devtools link in a Web dev session.";

fn flag(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(index) = args.iter().position(|arg| arg == name) {
        args.remove(index);

        true
    } else {
        false
    }
}

fn option(args: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    let Some(index) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };

    args.remove(index);

    if index == args.len() || args[index].starts_with("--") {
        return Err(format!("{name} requires a value").into());
    }

    Ok(Some(args.remove(index)))
}

fn parse_id<T: FromStr>(value: &str) -> Result<T>
where
    T::Err: Error + 'static,
{
    Ok(value.parse()?)
}

fn mode(value: &str) -> Result<TabMode> {
    match value {
        "foreground" => Ok(TabMode::Foreground),
        "live" => Ok(TabMode::Live),
        _ => Err("mode must be foreground or live".into()),
    }
}

fn address(route: &str, args: &mut Vec<String>) -> Result<RouteAddress> {
    let mut params = std::collections::BTreeMap::new();

    while let Some(value) = option(args, "--param")? {
        let (key, value) = value.split_once('=').ok_or("--param expects name=value")?;

        if key.is_empty() || params.insert(key.into(), value.into()).is_some() {
            return Err("duplicate or empty route parameter".into());
        }
    }

    Ok(RouteAddress {
        route: route.into(),
        params,
    })
}

fn parse_message(mut args: Vec<String>) -> Result<(Message, bool)> {
    if args.is_empty() {
        return Err(HELP.into());
    }

    let topic = args.remove(0);

    let action = if args.is_empty() {
        String::new()
    } else {
        args.remove(0)
    };

    // Unlike the ordinary two-word topics, bare quit accepts its own options.
    if topic == "quit" && !action.is_empty() && action != "status" {
        args.insert(0, action.clone());
    }

    let mut follow = false;

    let message = match (topic.as_str(), action.as_str()) {
        ("request", _) => {
            let message = serde_json::from_str(&action)?;

            if !args.is_empty() {
                return Err("unexpected request arguments".into());
            }

            return Ok((message, false));
        }
        ("quit", "status") => Message::Query(Query::Quit {}),
        ("quit", "" | "--jobs") => {
            let jobs = match option(&mut args, "--jobs")?.as_deref().unwrap_or("wait") {
                "wait" => rake_protocol::quit::QuitJobs::Wait,
                "cancel" => rake_protocol::quit::QuitJobs::Cancel,
                _ => return Err("quit jobs must be wait or cancel".into()),
            };

            Message::Command(Command::Quit {
                options: rake_protocol::quit::QuitOptions { jobs },
            })
        }
        ("reload", "status") => Message::Query(Query::Reload {}),
        ("reload", "checkpoint") if args.len() == 1 => Message::Query(Query::Checkpoint {
            application_id: args.remove(0),
        }),
        ("reload", "prepare") => {
            let mechanism = option(&mut args, "--mechanism")?.unwrap_or("native_restart".into());

            if args.len() != 1 {
                return Err("reload prepare requires an ID".into());
            }

            Message::Command(Command::PrepareReload {
                request: rake_protocol::reload::ReloadRequest {
                    id: args.remove(0),
                    mechanism: serde_json::from_value(json!(mechanism))?,
                },
            })
        }
        ("reload", "apply" | "abort") if args.len() == 1 => {
            Message::Command(Command::FinishReload {
                id: args.remove(0),
                applied: action == "apply",
            })
        }
        ("highlight", _) if !action.is_empty() => Message::Command(Command::HighlightElement {
            id: (action != "none").then_some(action),
            generation: option(&mut args, "--generation")?.unwrap_or_default(),
        }),
        ("inspect", "pick") => Message::Query(Query::InspectPick {}),
        ("inspect", "focus" | "scroll") => {
            let generation = option(&mut args, "--generation")?.unwrap_or_default();
            if args.len() != 1 {
                return Err("inspect focus|scroll requires one exact element ID".into());
            }
            Message::Command(Command::NavigateElement {
                id: args.remove(0),
                generation,
                action: if action == "focus" {
                    rake_protocol::inspection::NavigationAction::Focus
                } else {
                    rake_protocol::inspection::NavigationAction::ScrollIntoView
                },
            })
        }
        ("inspect", "mode") if !args.is_empty() => {
            let enabled = match args.remove(0).as_str() {
                "on" => true,
                "off" => false,
                _ => return Err("inspect mode expects on or off".into()),
            };

            let lease_ms = option(&mut args, "--lease-ms")?
                .map(|value| value.parse())
                .transpose()?
                .unwrap_or(1000);

            Message::Command(Command::InspectMode { enabled, lease_ms })
        }
        ("inspect", "tree") => Message::Query(Query::InspectTree {}),
        ("inspect", "element") if args.len() == 1 => {
            Message::Query(Query::InspectElement { id: args.remove(0) })
        }
        ("info", "") => Message::Query(Query::Info {}),
        ("schema", "") => Message::Query(Query::Schema {}),
        ("snapshot", "") => Message::Query(Query::Snapshot {}),
        ("events", "") | ("events", "--follow") => {
            follow = action == "--follow";

            Message::Query(Query::Events { cursor: None })
        }
        ("commands", "list") => Message::Query(Query::Commands {}),
        ("commands", "run") => {
            if args.is_empty() {
                return Err("commands run requires an ID".into());
            }

            let id = args.remove(0);

            let arguments = serde_json::from_str(
                &option(&mut args, "--args")?.ok_or("command requires --args JSON")?,
            )?;

            Message::Command(Command::Run { id, arguments })
        }
        ("routes", "list") => Message::Query(Query::Routes {}),
        ("tabs", "list") => Message::Query(Query::Tabs {}),
        ("operations", "list") => Message::Query(Query::Operations {}),
        ("decisions", "list") => Message::Query(Query::Decisions {}),
        ("tasks", "list") => {
            let owner = option(&mut args, "--owner")?
                .map(|owner| -> Result<rake_protocol::tasks::TaskOwner> {
                    if owner == "application" {
                        Ok(rake_protocol::tasks::TaskOwner::Application)
                    } else if owner.starts_with("tab-") {
                        Ok(rake_protocol::tasks::TaskOwner::Tab(parse_id(&owner)?))
                    } else {
                        Ok(rake_protocol::tasks::TaskOwner::Entry(parse_id(&owner)?))
                    }
                })
                .transpose()?;

            Message::Query(Query::Tasks { owner })
        }
        ("logs", "list" | "follow") | ("requests", "list") | ("traces", "export") => {
            follow = action == "follow";
            let after = option(&mut args, "--after")?
                .map(|value| value.parse())
                .transpose()?
                .unwrap_or(0);

            let kind = match topic.as_str() {
                "requests" => Some("request".into()),
                "traces" => Some("span".into()),
                _ => None,
            };

            Message::Query(Query::Diagnostics { after, kind })
        }
        ("windows", "list") => Message::Query(Query::Windows {}),
        ("windows", "get" | "close") if args.len() == 1 => {
            let window = parse_id(&args.remove(0))?;

            if action == "get" {
                Message::Query(Query::Window { window })
            } else {
                Message::Command(Command::CloseWindow { window })
            }
        }
        ("router", "open" | "reveal") => {
            if args.is_empty() {
                return Err("router operation requires a route key".into());
            }

            let route = args.remove(0);
            let destination = address(&route, &mut args)?;

            let window = option(&mut args, "--window")?
                .map(|value| parse_id(&value))
                .transpose()?;

            if action == "reveal" {
                Message::Command(Command::Reveal {
                    window,
                    scope: match option(&mut args, "--scope")?
                        .as_deref()
                        .unwrap_or("application")
                    {
                        "application" => rake_protocol::router::RevealScope::Application,
                        "window" => rake_protocol::router::RevealScope::Window,
                        _ => return Err("unknown reveal scope".into()),
                    },
                    destination,
                })
            } else {
                let target = match option(&mut args, "--target")?
                    .as_deref()
                    .unwrap_or("new-tab")
                {
                    "new-tab" => OpenTarget::NewTab,
                    "current-tab" => OpenTarget::CurrentTab,
                    "new-window" => OpenTarget::NewWindow,
                    _ => return Err("unknown open target".into()),
                };

                let mode = option(&mut args, "--mode")?
                    .map(|value| mode(&value))
                    .transpose()?;

                let activation = if flag(&mut args, "--preserve-selection") {
                    Activation::PreserveSelection
                } else {
                    Activation::Activate
                };

                Message::Command(Command::Open {
                    window,
                    destination,
                    options: OpenOptions {
                        target,
                        activation,
                        mode,
                    },
                })
            }
        }
        ("tabs", "select" | "close" | "back" | "forward" | "mode" | "navigate" | "replace") => {
            if args.is_empty() {
                return Err("tab operation requires a tab ID".into());
            }

            let tab = parse_id(&args.remove(0))?;

            let command = match action.as_str() {
                "select" => Command::Select { tab },
                "close" => Command::Close { tab },
                "back" => Command::Back { tab },
                "forward" => Command::Forward { tab },
                "mode" => {
                    if args.is_empty() {
                        return Err("tabs mode requires foreground or live".into());
                    }

                    Command::Mode {
                        tab,
                        mode: mode(&args.remove(0))?,
                    }
                }
                _ => {
                    if args.is_empty() {
                        return Err("navigation requires a route key".into());
                    }

                    let route = args.remove(0);

                    Command::Navigate {
                        tab,
                        destination: address(&route, &mut args)?,
                        replace: action == "replace",
                    }
                }
            };

            Message::Command(command)
        }
        ("tasks", "cancel") | ("operations", "get" | "watch" | "cancel") => {
            if args.is_empty() {
                return Err("operation requires an ID".into());
            }

            let id = args.remove(0);

            if topic == "tasks" {
                Message::Command(Command::CancelTask {
                    task: parse_id(&id)?,
                })
            } else {
                let operation = parse_id(&id)?;

                if action == "cancel" {
                    Message::Command(Command::CancelOperation { operation })
                } else {
                    follow = action == "watch";

                    Message::Query(Query::Operation { operation })
                }
            }
        }
        ("decisions", "respond") => {
            if args.is_empty() {
                return Err("response requires a decision ID".into());
            }

            let decision = parse_id(&args.remove(0))?;
            let choice = option(&mut args, "--choice")?.ok_or("response requires --choice")?;

            Message::Command(Command::Respond { decision, choice })
        }
        _ => return Err(format!("unsupported command\n{HELP}").into()),
    };

    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")).into());
    }

    Ok((message, follow))
}

pub(crate) fn exchange(instance: &Instance, request: &Request, timeout: Duration) -> Result<Reply> {
    Ok(rake_client::exchange(instance, request, timeout)?)
}

pub(crate) fn query(instance: &Instance, query: Query, timeout: Duration) -> Result<Reply> {
    Ok(rake_client::query(instance, query, timeout)?)
}

fn print(value: &impl serde::Serialize, compact: bool) -> Result<()> {
    if compact {
        println!("{}", serde_json::to_string(value)?);
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
    }

    Ok(())
}

pub(super) fn run(mut args: Vec<String>) -> Result<()> {
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{HELP}");

        return Ok(());
    }

    let compact = flag(&mut args, "--json");

    let timeout: f64 = option(&mut args, "--timeout")?
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(15.0);

    if !timeout.is_finite() || !(0.01..=3600.0).contains(&timeout) {
        return Err("timeout must be 0.01–3600 seconds".into());
    }

    let timeout = Duration::from_secs_f64(timeout);

    if args.is_empty() {
        return Err(HELP.into());
    }

    if args
        .first()
        .is_some_and(|arg| matches!(arg.as_str(), "highlight" | "inspect"))
    {
        args.insert(0, "ctl".into());
    }

    let directory = option(&mut args, "--session")?
        .map(|path| std::path::PathBuf::from(path).join("control"))
        .unwrap_or_else(local::directory);

    let instances = local::discover(&directory)?;
    let topic = args.remove(0);
    let selector = option(&mut args, "--instance")?;

    if topic == "instances" {
        if !args.is_empty() {
            return Err("unexpected instances arguments".into());
        }

        let mut live = Vec::new();

        for instance in &instances {
            if matches!(query(instance, Query::Info {}, Duration::from_secs(2)), Ok(Reply::Info(info)) if info.runtime_id == instance.info.runtime_id)
            {
                live.push(json!({"application_id": instance.application_id, "pid": instance.pid, "info": instance.info}));
            }
        }

        return print(&live, compact);
    }

    if topic != "ctl" {
        return Err(format!("unsupported command\n{HELP}").into());
    }

    let no_wait = flag(&mut args, "--no-wait");

    let key =
        option(&mut args, "--request-key")?.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let friendly_highlight = args.first().is_some_and(|arg| arg == "highlight");
    let friendly_navigation = args.first().is_some_and(|arg| arg == "inspect")
        && args
            .get(1)
            .is_some_and(|arg| matches!(arg.as_str(), "focus" | "scroll"));
    let source = args.first().is_some_and(|arg| arg == "inspect")
        && args.get(1).is_some_and(|arg| arg == "source");
    let mut source_target = rake_protocol::inspection::SourceTarget::Boundary {};
    if source {
        let style = option(&mut args, "--style")?;
        let handler = option(&mut args, "--handler")?;
        source_target = match (style, handler) {
            (Some(_), Some(_)) => return Err("choose either --style or --handler".into()),
            (Some(index), None) => rake_protocol::inspection::SourceTarget::Style {
                index: index.parse()?,
            },
            (None, Some(index)) => rake_protocol::inspection::SourceTarget::Handler {
                index: index.parse()?,
            },
            _ => rake_protocol::inspection::SourceTarget::Boundary {},
        };
        args[1] = "element".into();
    }
    let (mut message, follow) = parse_message(args)?;

    let candidates: Vec<_> = instances
        .iter()
        .filter(|instance| {
            selector.as_ref().is_none_or(|selector| {
                *selector == instance.info.runtime_id.to_string()
                    || *selector == instance.application_id
            })
        })
        .collect();

    let selected: Vec<_> = candidates.into_iter().filter(|instance| matches!(query(instance, Query::Info {}, timeout), Ok(Reply::Info(info)) if info.runtime_id == instance.info.runtime_id)).collect();

    let instance = match selected.as_slice() {
        [] => return Err("no matching development instance; use rake instances --json".into()),
        [instance] => *instance,
        _ => return Err("ambiguous instance; select a RuntimeId using --instance".into()),
    };

    if let Message::Command(Command::HighlightElement {
        id: Some(id),
        generation,
    }) = &mut message
        && generation.is_empty()
        && friendly_highlight
    {
        let Reply::Elements(tree) = query(instance, Query::InspectTree {}, timeout)? else {
            return Err("unexpected inspection reply".into());
        };

        if !tree.elements.iter().any(|element| element.id == *id) {
            return Err("element is stale; refresh inspect tree".into());
        }

        *generation = tree.capture;
    }

    if let Message::Command(Command::NavigateElement { id, generation, .. }) = &mut message
        && generation.is_empty()
        && friendly_navigation
    {
        let Reply::Elements(tree) = query(instance, Query::InspectTree {}, timeout)? else {
            return Err("Unexpected inspection reply".into());
        };
        if !tree.elements.iter().any(|element| element.id == *id) {
            return Err("Element is stale; refresh inspect tree".into());
        }
        *generation = tree.capture;
    }

    let request = Request {
        protocol: PROTOCOL_VERSION,
        runtime_id: instance.info.runtime_id,
        key: Some(key.clone()),
        message: message.clone(),
    };

    let reply = exchange(instance, &request, timeout).map_err(|error| {
        if matches!(message, Message::Command(_)) { format!("{error}; request key {key}; retry the identical payload and key if the response was lost") }
        else { error.to_string() }
    })?;

    if source {
        let Reply::Element(element) = reply else {
            return Err("Unexpected source lookup reply".into());
        };
        let location = element
            .source_location(&source_target)
            .ok_or("Source location unavailable")?;
        return print(&crate::source::open(location)?, compact);
    }

    let deadline = Instant::now() + timeout;

    if let Reply::Submitted { operation_id } = reply {
        if no_wait || matches!(message, Message::Command(Command::Quit { .. })) {
            return print(&json!({"request_key": key, "reply": reply}), compact);
        }

        loop {
            let reply = query(
                instance,
                Query::Operation {
                    operation: operation_id,
                },
                timeout,
            )?;

            if let Reply::Operation(record) = &reply
                && !matches!(
                    record.state,
                    OperationState::Queued | OperationState::Running
                )
            {
                print(&json!({"request_key": key, "reply": reply}), compact)?;

                if matches!(record.state, OperationState::AwaitingDecision(_)) {
                    print(&query(instance, Query::Decisions {}, timeout)?, compact)?;
                }

                return Ok(());
            }

            if Instant::now() >= deadline {
                return Err(format!("operation {operation_id} still pending; request key {key}; observation timed out").into());
            }

            std::thread::sleep(Duration::from_millis(50));
        }
    }

    print(&reply, compact)?;

    if follow {
        let mut previous = serde_json::to_value(&reply)?;

        let mut next_query = match message {
            Message::Query(query) => query,
            _ => return Ok(()),
        };

        let mut last_reply = reply;

        loop {
            match (&mut next_query, &last_reply) {
                (Query::Events { cursor }, Reply::Event(event)) => *cursor = Some(event.cursor),
                (Query::Diagnostics { after, .. }, Reply::Diagnostics(snapshot)) => {
                    if let Some(record) = snapshot.records.last() {
                        *after = record.sequence;
                    }
                }
                (Query::Operation { .. }, Reply::Operation(record))
                    if !matches!(
                        record.state,
                        OperationState::Queued
                            | OperationState::Running
                            | OperationState::AwaitingDecision(_)
                    ) =>
                {
                    break;
                }
                _ => {}
            }

            if Instant::now() >= deadline {
                return Err("watch timed out; runtime work continues".into());
            }

            std::thread::sleep(Duration::from_millis(100));
            last_reply = query(instance, next_query.clone(), timeout)?;
            let value = serde_json::to_value(&last_reply)?;

            let quiet = matches!(&last_reply, Reply::Event(event) if event.kind == rake_protocol::control::EventKind::Unchanged)
                || matches!(&last_reply, Reply::Diagnostics(snapshot) if snapshot.records.is_empty());

            if !quiet && value != previous {
                print(&last_reply, compact)?;
            }

            previous = value;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    //! Covers: terminal-to-protocol mapping and malformed/unsupported arguments.
    //! Real: argument parser and shared Serde schema. Doubles: none. Requires: none.
    use super::*;

    #[test]
    fn maps_owned_addresses_and_independent_open_options() {
        let (message, _) = parse_message(
            [
                "router",
                "open",
                "document",
                "--param",
                "id=42",
                "--preserve-selection",
                "--mode",
                "live",
            ]
            .map(String::from)
            .to_vec(),
        )
        .unwrap();

        let Message::Command(Command::Open {
            window: None,
            destination,
            options,
        }) = message
        else {
            panic!()
        };

        assert_eq!(destination.params["id"], "42");
        assert_eq!(options.activation, Activation::PreserveSelection);
        assert_eq!(options.mode, Some(TabMode::Live));
        assert!(
            parse_message(
                [
                    "router", "open", "document", "--param", "id=42", "--param", "id=9"
                ]
                .map(String::from)
                .to_vec()
            )
            .is_err()
        );
        assert!(matches!(
            parse_message(["inspect", "tree"].map(String::from).to_vec())
                .unwrap()
                .0,
            Message::Query(Query::InspectTree {})
        ));
        assert!(parse_message(["inspect", "element"].map(String::from).to_vec()).is_err());

        for (arguments, jobs) in [
            (vec!["quit"], rake_protocol::quit::QuitJobs::Wait),
            (
                vec!["quit", "--jobs", "wait"],
                rake_protocol::quit::QuitJobs::Wait,
            ),
            (
                vec!["quit", "--jobs", "cancel"],
                rake_protocol::quit::QuitJobs::Cancel,
            ),
        ] {
            assert!(
                matches!(parse_message(arguments.into_iter().map(String::from).collect()).unwrap().0,
                Message::Command(Command::Quit { options }) if options.jobs == jobs)
            );
        }

        assert!(matches!(
            parse_message(vec!["quit".into(), "status".into()])
                .unwrap()
                .0,
            Message::Query(Query::Quit {})
        ));

        for arguments in [
            vec!["quit", "--jobs"],
            vec!["quit", "--jobs", "force"],
            vec!["quit", "status", "--jobs", "wait"],
        ] {
            assert!(parse_message(arguments.into_iter().map(String::from).collect()).is_err());
        }
    }

    #[test]
    fn every_advertised_wire_tag_has_a_raw_terminal_path() {
        let tab = "tab-1".parse().unwrap();
        let operation = "op-1".parse().unwrap();

        let destination = RouteAddress {
            route: "home".into(),
            params: Default::default(),
        };

        let commands = vec![
            Command::RecordMemory {
                owner: "test".into(),
                enabled: false,
                interval: 32768,
                lease_ms: 0,
            },
            Command::EditElementText {
                id: "boundary".into(),
                generation: "1".into(),
                edit: rake_protocol::inspection::TextEdit::Reset {},
            },
            Command::RecordFrames {
                enabled: false,
                lease_ms: 0,
            },
            Command::EditElementStyle {
                id: "boundary".into(),
                generation: "1".into(),
                edit: rake_protocol::inspection::StyleEdit::Reset,
            },
            Command::NavigateElement {
                id: "element-1-0".into(),
                generation: "1".into(),
                action: rake_protocol::inspection::NavigationAction::Focus,
            },
            Command::HighlightElement {
                id: None,
                generation: String::new(),
            },
            Command::InspectMode {
                enabled: false,
                lease_ms: 0,
            },
            Command::CloseWindow {
                window: "window-1".parse().unwrap(),
            },
            Command::Quit {
                options: rake_protocol::quit::QuitOptions::default(),
            },
            Command::PrepareReload {
                request: rake_protocol::reload::ReloadRequest {
                    id: "test".into(),
                    mechanism: rake_protocol::reload::ReloadMechanism::NativeRestart,
                },
            },
            Command::FinishReload {
                id: "test".into(),
                applied: false,
            },
            Command::Open {
                window: None,
                destination: destination.clone(),
                options: OpenOptions::new_tab(),
            },
            Command::Navigate {
                tab,
                destination: destination.clone(),
                replace: false,
            },
            Command::Reveal {
                window: None,
                scope: rake_protocol::router::RevealScope::Application,
                destination,
            },
            Command::Select { tab },
            Command::Close { tab },
            Command::Back { tab },
            Command::Forward { tab },
            Command::Mode {
                tab,
                mode: TabMode::Live,
            },
            Command::CancelOperation { operation },
            Command::Respond {
                decision: "decision-1".parse().unwrap(),
                choice: "cancel".into(),
            },
            Command::CancelTask {
                task: "task-1".parse().unwrap(),
            },
            Command::Run {
                id: "example".into(),
                arguments: json!({}),
            },
        ];

        let queries = vec![
            Query::Memory {
                owner: "test".into(),
                include_freed: false,
            },
            Query::InspectPick {},
            Query::Frames { after: 0 },
            Query::Windows {},
            Query::Window {
                window: "window-1".parse().unwrap(),
            },
            Query::Quit {},
            Query::InspectTree {},
            Query::InspectTreePage {
                generation: None,
                offset: 0,
            },
            Query::InspectElement {
                id: "element-1-0".into(),
            },
            Query::Reload {},
            Query::Checkpoint {
                application_id: "test".into(),
            },
            Query::Info {},
            Query::Schema {},
            Query::Routes {},
            Query::Commands {},
            Query::Snapshot {},
            Query::Tabs {},
            Query::Tasks { owner: None },
            Query::Operations {},
            Query::Operation { operation },
            Query::Decisions {},
            Query::Events { cursor: None },
            Query::HttpBody {
                request_id: 1,
                response: true,
            },
            Query::Diagnostics {
                after: 0,
                kind: None,
            },
        ];

        let mut command_tags = std::collections::BTreeSet::new();
        let mut query_tags = std::collections::BTreeSet::new();

        for message in commands
            .into_iter()
            .map(Message::Command)
            .chain(queries.into_iter().map(Message::Query))
        {
            let value = serde_json::to_value(&message).unwrap();
            let tag = value["value"]["type"].as_str().unwrap().to_owned();

            match message {
                Message::Command(_) => {
                    command_tags.insert(tag);
                }
                Message::Query(_) => {
                    query_tags.insert(tag);
                }
            }

            let (parsed, _) = parse_message(vec!["request".into(), value.to_string()]).unwrap();

            assert_eq!(serde_json::to_value(parsed).unwrap(), value);
        }

        assert_eq!(
            command_tags,
            Command::names()
                .iter()
                .map(|name| (*name).to_owned())
                .collect()
        );
        assert_eq!(
            query_tags,
            Query::names()
                .iter()
                .map(|name| (*name).to_owned())
                .collect()
        );

        assert!(
            parse_message(vec![
                "request".into(),
                r#"{"type":"command","value":{"type":"shell","command":"ls"}}"#.into()
            ])
            .is_err()
        );
    }
}
