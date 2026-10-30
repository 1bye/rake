//! Explicit typed-command evaluation. One submission, fresh retry key, no automatic resend.

use super::wire::Error;
use rake_client::local::Instance;
use rake_protocol::control::{Command, Message, PROTOCOL_VERSION, Query, Reply, Request};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub(super) fn parse(expression: &str) -> Result<(String, Value), Error> {
    let expression = expression.trim();

    let (id, arguments) = expression
        .split_once('(')
        .ok_or_else(|| Error::new(-32602, "Use a registered command: name(JSON)"))?;

    let id = id.trim();

    if id.is_empty()
        || id.len() > 128
        || id
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '(' | ')' | ';'))
    {
        return Err(Error::new(-32602, "Use a registered command: name(JSON)"));
    }

    let arguments = arguments
        .strip_suffix(')')
        .ok_or_else(|| Error::new(-32602, "Expected name(JSON), with no trailing code"))?;

    let arguments = serde_json::from_str(arguments)
        .map_err(|e| Error::new(-32602, format!("Command arguments must be JSON: {e}")))?;

    Ok((id.into(), arguments))
}

pub(super) fn registry(instance: &Instance) -> Result<Value, Error> {
    if !instance
        .info
        .queries
        .iter()
        .any(|query| query == "commands")
    {
        return Ok(json!([]));
    }
    match crate::terminal::query(instance, Query::Commands {}, super::TIMEOUT)
        .map_err(|e| Error::new(-32000, e.to_string()))?
    {
        Reply::Data(value) if value.is_array() => Ok(value),
        _ => Err(Error::new(-32000, "Invalid command registry")),
    }
}

pub(super) fn names(instance: &Instance) -> Result<Vec<String>, Error> {
    Ok(registry(instance)?
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str().map(str::to_owned))
        .collect())
}

pub(super) fn help() -> Value {
    json!({"help":"rake.help()", "commands":"rake.commands()", "command":"rake.command(\"demo.echo\") — argument/result schemas",
        "operation":"rake.operation({\"runtimeId\":\"…\",\"id\":\"op-1\"}) — read a pending outcome without resubmitting",
        "memory":"rake.memory() — native allocation counts, estimates and capture limits", "stopMemory":"rake.stopMemory() — stop tracking and clear native profiles",
        "sources":"rake.sources() — captured files and catalog limits", "source":"rake.source($0) — captured file URL and location", "openSource":"rake.openSource($0) or rake.openSource({url,lineNumber,columnNumber})","resetStyles":"rake.resetStyles($0)","undoStyles":"rake.undoStyles($0)",
        "resetText":"rake.resetText($0)","undoText":"rake.undoText($0)",
        "evaluation":"Registered commands accept one JSON argument. General JavaScript and Rust evaluation are unavailable."})
}

pub(super) fn completion(expression: &str, names: &[String]) -> Option<Value> {
    let helpers = help();
    if expression == "rake" {
        return Some(helpers);
    }
    let prefix = format!("{expression}.");
    let mut properties = serde_json::Map::new();
    for name in names {
        if let Some(suffix) = name.strip_prefix(&prefix) {
            let child = suffix.split('.').next().unwrap();
            properties.insert(child.into(), json!(format!("{name}(JSON)")));
        }
    }
    (!properties.is_empty()).then_some(Value::Object(properties))
}

pub(super) fn lexical_names(names: &[String]) -> Vec<String> {
    let mut result = std::collections::BTreeSet::from(["rake".to_owned(), "$0".to_owned()]);
    for name in names {
        result.insert(name.split('.').next().unwrap().to_owned());
    }
    result.into_iter().collect()
}

pub(super) fn observation(instance: &Instance, argument: &Value) -> Result<Value, Error> {
    if argument["runtimeId"].as_str() != Some(&instance.info.runtime_id.to_string()) {
        return Err(Error::new(
            -32602,
            "Operation belongs to a different runtime; use the returned observation token",
        ));
    }
    let operation = argument["id"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| Error::new(-32602, "Expected {runtimeId,id:\"op-N\"}"))?;
    match crate::terminal::query(instance, Query::Operation { operation }, super::TIMEOUT)
        .map_err(|error| Error::new(-32000, error.to_string()))?
    {
        Reply::Operation(record) => Ok(outcome(instance, None, &record)),
        _ => Err(Error::new(-32000, "Unexpected operation reply")),
    }
}

fn outcome(
    instance: &Instance,
    key: Option<&str>,
    record: &rake_protocol::control::OperationRecord,
) -> Value {
    use rake_protocol::router::OperationState;
    let status = match &record.state {
        OperationState::Committed => "committed",
        OperationState::Queued => "queued",
        OperationState::Running => "running",
        OperationState::AwaitingDecision(_) => "awaiting decision",
        OperationState::Cancelled(_) => "cancelled",
        OperationState::Blocked(_) => "blocked",
        OperationState::Failed(_) => "failed",
    };
    json!({"requestKey":key,"status":status,"operation":record,"observe":{"runtimeId":instance.info.runtime_id.to_string(),"id":record.operation_id.to_string()}})
}

pub(super) fn validate(instance: &Instance, expression: &str) -> Result<(String, Value), Error> {
    let (id, arguments) = parse(expression)?;

    if !names(instance)?.contains(&id) {
        return Err(Error::new(-32602, format!("Unregistered command: {id}")));
    }

    Ok((id, arguments))
}

pub(super) fn execute(instance: &Instance, expression: &str) -> Result<Value, Error> {
    let (id, arguments) = validate(instance, expression)?;
    let key = uuid::Uuid::new_v4().to_string();

    let request = Request {
        protocol: PROTOCOL_VERSION,
        runtime_id: instance.info.runtime_id,
        key: Some(key.clone()),
        message: Message::Command(Command::Run { id, arguments }),
    };

    let reply = crate::terminal::exchange(instance, &request, super::TIMEOUT)
        .map_err(|e| delivery_error(e.to_string(), &key))?;

    let Reply::Submitted { operation_id } = reply else {
        return Err(delivery_error("Unexpected command submission reply", &key));
    };

    let deadline = Instant::now() + super::TIMEOUT;

    loop {
        match crate::terminal::query(
            instance,
            Query::Operation {
                operation: operation_id,
            },
            super::TIMEOUT,
        ) {
            Ok(Reply::Operation(record)) => {
                use rake_protocol::router::OperationState;

                match record.state {
                    OperationState::Committed => {
                        return Ok(outcome(instance, Some(&key), &record));
                    }
                    OperationState::Queued | OperationState::Running
                        if Instant::now() < deadline => {}
                    _ => return Ok(outcome(instance, Some(&key), &record)),
                }
            }
            Ok(_) => {
                return Err(delivery_error(
                    format!("Unexpected operation reply for {operation_id}"),
                    &key,
                ));
            }
            Err(e) => {
                return Err(delivery_error(
                    format!("Operation {operation_id} observation failed: {e}"),
                    &key,
                ));
            }
        }

        std::thread::sleep(Duration::from_millis(20));
    }
}

fn delivery_error(message: impl std::fmt::Display, key: &str) -> Error {
    Error::new(
        -32000,
        format!(
            "{message}; request key {key}; delivery/outcome may be uncertain. This evaluation was not resent. Query the operation or explicitly retry the identical command/key through rake ctl; evaluating again creates a new key."
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_is_static_and_outcomes_carry_runtime_scoped_observation_tokens() {
        let names = vec!["demo.echo".into(), "demo.files.list".into()];
        assert_eq!(lexical_names(&names), ["$0", "demo", "rake"]);
        let namespace = completion("demo", &names).unwrap();
        assert_eq!(namespace["echo"], "demo.echo(JSON)");
        assert!(namespace.get("files").is_some());
        assert!(completion("demo.echo({})", &names).is_none());
        assert!(help()["resetText"].as_str().unwrap().contains("$0"));
        let router =
            rake::router::Router::<rake_protocol::router::NoRoute>::new(Default::default());
        let instance = Instance {
            application_id: "observe".into(),
            pid: 0,
            info: router.client().info(),
            endpoint: rake_client::local::Endpoint::Local {
                path: "/never-connect".into(),
            },
        };
        let record = rake_protocol::control::OperationRecord {
            operation_id: rake_protocol::router::OperationId(3),
            state: rake_protocol::router::OperationState::AwaitingDecision(
                rake_protocol::router::DecisionId(9),
            ),
            result: None,
        };
        let result = outcome(&instance, Some("key"), &record);
        assert_eq!(result["status"], "awaiting decision");
        assert_eq!(result["observe"]["id"], "op-3");
        assert_eq!(result["requestKey"], "key");
        assert!(observation(&instance, &json!({"runtimeId":"old","id":"op-3"})).is_err());
    }

    #[test]
    fn inspection_only_hosts_offer_no_typed_command_completion_or_evaluation() {
        let router = rake::router::Router::<rake_protocol::router::NoRoute>::new(
            rake::router::RouterConfig::default(),
        );
        let mut info = router.client().info();
        info.queries = vec!["info".into(), "inspect_tree".into()];
        let instance = Instance {
            application_id: "standalone".into(),
            pid: 0,
            info,
            endpoint: rake_client::local::Endpoint::Local {
                path: "/no/inspection-socket".into(),
            },
        };
        assert!(names(&instance).unwrap().is_empty());
        assert!(
            validate(&instance, "app.command(null)")
                .unwrap_err()
                .message
                .contains("Unregistered command")
        );
    }

    #[test]
    fn accepts_only_one_command_and_json_arguments() {
        assert_eq!(
            parse(" demo.echo({\"message\":\"a(b)\"}) ").unwrap(),
            ("demo.echo".into(), json!({"message":"a(b)"}))
        );
        assert_eq!(parse("unit(null)").unwrap().1, Value::Null);

        for invalid in [
            "1+1",
            "demo.echo({message:'bad'})",
            "demo.echo({}); other()",
            "demo.echo()",
            "demo.echo({}) trailing",
            "bad name({})",
        ] {
            assert!(parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn lost_submission_reply_names_key_and_never_resends() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        let path =
            std::path::PathBuf::from(format!("/tmp/rake-eval-{}.sock", uuid::Uuid::new_v4()));

        let listener = UnixListener::bind(&path).unwrap();

        listener.set_nonblocking(true).unwrap();
        let router = rake::router::Router::<rake_protocol::router::NoRoute>::new(
            rake::router::RouterConfig::default(),
        );

        let instance = Instance {
            application_id: "test.eval".into(),
            pid: std::process::id(),
            info: router.client().info(),
            endpoint: rake_client::local::Endpoint::Local { path: path.clone() },
        };

        let (done, finished) = std::sync::mpsc::channel();

        let worker = std::thread::spawn(move || {
            let mut keys = Vec::new();

            loop {
                let (mut socket, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        match finished.try_recv() {
                            Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                            Err(std::sync::mpsc::TryRecvError::Empty) => {}
                        }

                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };

                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut line = String::new();

                BufReader::new(socket.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: Request = serde_json::from_str(&line).unwrap();

                match request.message {
                    Message::Query(Query::Commands {}) => {
                        let reply = Ok::<_, rake_protocol::control::ControlError>(Reply::Data(
                            json!([{"id":"test.echo"}]),
                        ));

                        writeln!(socket, "{}", serde_json::to_string(&reply).unwrap()).unwrap();
                    }
                    Message::Command(Command::Run { .. }) => {
                        keys.push(request.key.unwrap());
                    }
                    _ => panic!("unexpected fixture request"),
                }
            }

            keys
        });

        let error = execute(&instance, "test.echo({\"message\":\"one\"})").unwrap_err();

        done.send(()).unwrap();
        let keys = worker.join().unwrap();

        assert_eq!(keys.len(), 1);
        assert!(error.message.contains(&keys[0]));
        assert!(error.message.contains("not resent"));
        std::fs::remove_file(path).unwrap();
    }
}
