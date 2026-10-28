//! Explicit typed-command evaluation. One submission, fresh retry key, no automatic resend.

use super::wire::Error;
use rake::control::local::Instance;
use rake::control::{Command, Message, PROTOCOL_VERSION, Query, Reply, Request};
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

pub(super) fn names(instance: &Instance) -> Result<Vec<String>, Error> {
    let reply = crate::terminal::query(instance, Query::Commands {}, super::TIMEOUT)
        .map_err(|e| Error::new(-32000, e.to_string()))?;
    let Reply::Data(value) = reply else {
        return Err(Error::new(-32000, "Unexpected command registry reply"));
    };
    let commands = value
        .as_array()
        .ok_or_else(|| Error::new(-32000, "Invalid command registry"))?;

    Ok(commands
        .iter()
        .filter_map(|c| c["id"].as_str().map(str::to_owned))
        .collect())
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
                use rake::router::OperationState;
                match record.state {
                    OperationState::Committed => {
                        return Ok(json!({"requestKey":key,"operation":record}));
                    }
                    OperationState::Queued | OperationState::Running
                        if Instant::now() < deadline => {}
                    _ => return Ok(json!({"requestKey":key,"operation":record})),
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
        let router = rake::router::Router::<rake::router::NoRoute>::new(
            rake::router::RouterConfig::default(),
        );
        let instance = Instance {
            application_id: "test.eval".into(),
            pid: std::process::id(),
            info: router.client().info(),
            endpoint: rake::control::local::Endpoint::Local { path: path.clone() },
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
                        let reply = Ok::<_, rake::control::ControlError>(Reply::Data(
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
