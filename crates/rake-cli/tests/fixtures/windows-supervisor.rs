//! Headless Windows app/helper fixture using the production private files and pipe transport.
use futures::{
    FutureExt, StreamExt,
    future::{Either, select},
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    stream::FuturesUnordered,
};
use rake_client::{
    pipe::{self, Listener, Stream},
    private,
};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    rc::Rc,
    time::Duration,
};

fn parked() -> ! {
    loop {
        std::thread::park();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("--leaf") => {
            if let Some(marker) = args.get(2) {
                std::fs::write(marker, std::process::id().to_string()).unwrap();
            }
            parked();
        }
        Some("--ready") => {
            println!(
                "{{\"event\":\"{}\",\"url\":\"http://127.0.0.1:1\"}}",
                args[2]
            );
            parked();
        }
        Some("--empty") => return,
        Some("--last-line") => {
            print!("{{\"event\":\"cdp_listening\"}}");
            std::io::stdout().flush().unwrap();
            return;
        }
        Some("--silent") => {
            std::fs::write(&args[2], std::process::id().to_string()).unwrap();
            parked();
        }
        Some("--overflow") => {
            std::io::stdout()
                .write_all(&vec![b'x'; rake_protocol::control::MAX_MESSAGE_BYTES + 1])
                .unwrap();
            parked();
        }
        _ => {}
    }

    let control = PathBuf::from(std::env::var_os("RAKE_CONTROL_DIR").expect("control directory"));
    let session = control.parent().unwrap().to_owned();
    let runtime = uuid::Uuid::new_v4();
    let checkpoint: Option<Value> = std::env::var_os("RAKE_RELOAD_CHECKPOINT")
        .map(|path| serde_json::from_reader(private::open(Path::new(&path)).unwrap()).unwrap());
    let generation = checkpoint
        .as_ref()
        .and_then(|value| value["generation"].as_u64())
        .map_or(1, |generation| generation + 1);
    let restored = checkpoint
        .as_ref()
        .and_then(|value| value["runtime"].as_str())
        .map(str::to_owned);
    let listener = Listener::bind(&pipe::path(&runtime)).unwrap();
    let manifest = json!({"application_id":"windows-fixture", "pid":std::process::id(),
        "info":{"protocol":1,"runtime_id":runtime,"platform":"native","commands":[],"queries":[],"supported_targets":[],"completed_operations":8,"pending_operations":8,"idempotency_capacity":8},
        "endpoint":{"type":"local","path":pipe::path(&runtime)}});
    private::write_atomic(
        &control.join(format!("{runtime}.json")),
        &serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let marker = session.join(format!("descendant-{}", std::process::id()));
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--leaf")
        .arg(&marker)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    println!("fixture output é漢字");
    eprintln!("fixture stderr");
    let state = Rc::new(State {
        generation,
        restored,
        runtime,
        prepared: Cell::new(false),
        session,
    });

    futures::executor::block_on(async {
        let mut clients = FuturesUnordered::new();
        loop {
            if clients.len() == 32 {
                clients.next().await;
                continue;
            }
            let stream = if clients.is_empty() {
                Some(listener.accept().await.unwrap())
            } else {
                match select(Box::pin(listener.accept()), Box::pin(clients.next())).await {
                    Either::Left((stream, _)) => Some(stream.unwrap()),
                    Either::Right(_) => None,
                }
            };
            if let Some(stream) = stream {
                clients.push(reply(stream, state.clone()).boxed_local());
            }
        }
    });
}

struct State {
    generation: u64,
    restored: Option<String>,
    runtime: uuid::Uuid,
    prepared: Cell<bool>,
    session: PathBuf,
}

async fn reply(stream: Stream, state: Rc<State>) {
    let mut reader =
        BufReader::new(stream.take(rake_protocol::control::MAX_MESSAGE_BYTES as u64 + 1));
    let mut input = Vec::new();
    if reader.read_until(b'\n', &mut input).await.is_err() {
        return;
    }
    let Ok(request) = serde_json::from_slice::<Value>(&input) else {
        return;
    };
    let value = &request["message"]["value"];
    let response = match value["type"].as_str().unwrap() {
        "prepare_reload" => {
            state.prepared.set(true);
            Value::Null
        }
        "finish_reload" => {
            state.prepared.set(false);
            Value::Null
        }
        "reload" => {
            json!({"phase":if state.prepared.get() {"ready"} else if state.restored.is_some() {"restored"} else {"idle"},"generation":state.generation,"restored_from":state.restored,"error":null})
        }
        "operations" => json!([]),
        "checkpoint" => {
            if state.session.join("stall-checkpoint").exists() {
                private::write_atomic(&state.session.join("checkpoint-started"), b"ready").unwrap();
                while state.session.join("stall-checkpoint").exists() {
                    async_io::Timer::after(Duration::from_millis(5)).await;
                }
            }
            json!({"generation":state.generation,"runtime":state.runtime,"draft":"é漢字"})
        }
        _ => Value::Null,
    };
    let mut bytes = serde_json::to_vec(&json!({"Ok":{"value":response}})).unwrap();
    bytes.push(b'\n');
    let _ = reader.get_mut().get_mut().write_all(&bytes).await;
    let mut end = [0];
    let _ = reader.get_mut().get_mut().read(&mut end).await;
}
