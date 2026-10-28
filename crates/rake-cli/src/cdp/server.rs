//! Bounded HTTP discovery and authenticated WebSocket sessions on IPv4 loopback.

use super::{Options, Result, discover, session};
use rake::control::local::Instance;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn allowed(host: Option<&str>, origin: Option<&str>, authority: &str) -> bool {
    host == Some(authority) && origin.is_none_or(|origin| origin == "devtools://devtools")
}

fn target(instance: &Instance, authority: &str, token: &str) -> Value {
    let id = instance.info.runtime_id.to_string();
    let path = format!("{authority}/{token}/{id}");

    json!({"id": id, "type": "page", "title": instance.application_id,
        "description": "Rake development runtime", "url": "rake://runtime",
        "webSocketDebuggerUrl": format!("ws://{path}"),
        "devtoolsFrontendUrl": format!("devtools://devtools/bundled/devtools_app.html?ws={path}")})
}

fn http(stream: &mut TcpStream, status: &str, body: &Value) -> std::io::Result<()> {
    let mut header = Vec::new();

    while !header.ends_with(b"\r\n\r\n") && header.len() < 16384 {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        header.push(byte[0]);
    }

    let body = serde_json::to_vec(body)?;
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)
}

fn connection(
    mut stream: TcpStream,
    options: &Options,
    authority: &str,
    token: &str,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut buffer = [0; 16384];
    let size = loop {
        let size = stream.peek(&mut buffer)?;

        if size == 0 {
            return Ok(());
        }
        if let Some(end) = buffer[..size].windows(4).position(|s| s == b"\r\n\r\n") {
            break end + 4;
        }
        if size == buffer.len() || Instant::now() >= deadline {
            return Err("oversized or incomplete HTTP header".into());
        }

        std::thread::sleep(Duration::from_millis(5));
    };
    let header = std::str::from_utf8(&buffer[..size])?;
    let mut lines = header.split("\r\n");
    let request = lines.next().ok_or("missing HTTP request")?;
    let parts: Vec<_> = request.split_whitespace().collect();

    if parts.len() != 3 || parts[0] != "GET" || parts[2] != "HTTP/1.1" {
        return Ok(http(
            &mut stream,
            "400 Bad Request",
            &json!({"error":"GET required"}),
        )?);
    }

    let mut host = None;
    let mut origin = None;

    for line in lines.filter(|s| !s.is_empty()) {
        let (key, value) = line.split_once(':').ok_or("invalid HTTP header")?;
        let slot = if key.eq_ignore_ascii_case("host") {
            Some(&mut host)
        } else if key.eq_ignore_ascii_case("origin") {
            Some(&mut origin)
        } else {
            None
        };

        if let Some(slot) = slot
            && slot.replace(value.trim()).is_some()
        {
            return Ok(http(
                &mut stream,
                "403 Forbidden",
                &json!({"error":"duplicate security header"}),
            )?);
        }
    }

    if !allowed(host, origin, authority) {
        return Ok(http(
            &mut stream,
            "403 Forbidden",
            &json!({"error":"Host or Origin rejected"}),
        )?);
    }

    let path = parts[1];

    if matches!(path, "/json" | "/json/list" | "/json/version") {
        let body = if path == "/json/version" {
            json!({"Browser":"Rake/0.1", "Protocol-Version":"1.3"})
        } else {
            json!(
                discover(&options.directory, options.selector.as_deref())?
                    .iter()
                    .map(|i| target(i, authority, token))
                    .collect::<Vec<_>>()
            )
        };
        return Ok(http(&mut stream, "200 OK", &body)?);
    }

    let Some(id) = path.strip_prefix(&format!("/{token}/")) else {
        return Ok(http(
            &mut stream,
            "403 Forbidden",
            &json!({"error":"session token required"}),
        )?);
    };
    let Some(instance) = discover(&options.directory, options.selector.as_deref())?
        .into_iter()
        .find(|i| i.info.runtime_id.to_string() == id)
    else {
        return Ok(http(
            &mut stream,
            "404 Not Found",
            &json!({"error":"runtime unavailable"}),
        )?);
    };
    let socket = tungstenite::accept_with_config(
        stream,
        Some(
            tungstenite::protocol::WebSocketConfig::default()
                .max_message_size(Some(1024 * 1024))
                .max_frame_size(Some(1024 * 1024)),
        ),
    )
    .map_err(|e| format!("CDP handshake: {e}"))?;

    session::run(socket, instance, options.process_output.clone())
}

pub(super) fn serve(options: Options) -> Result<()> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, options.port))?;
    let authority = listener.local_addr()?.to_string();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let targets: Vec<_> = discover(&options.directory, options.selector.as_deref())?
        .iter()
        .map(|i| target(i, &authority, &token))
        .collect();

    if options.json {
        println!(
            "{}",
            json!({"event":"cdp_listening", "url":format!("http://{authority}"), "port":listener.local_addr()?.port(), "targets":targets})
        );
    } else {
        println!("CDP discovery: http://{authority}/json/list");

        for target in &targets {
            println!(
                "{}",
                target["devtoolsFrontendUrl"].as_str().unwrap_or_default()
            );
        }
    }

    if options.open
        && let Some(url) = targets
            .first()
            .and_then(|t| t["devtoolsFrontendUrl"].as_str())
    {
        #[cfg(target_os = "macos")]
        let result = std::process::Command::new("open")
            .args(["-a", "Google Chrome", url])
            .status();
        #[cfg(not(target_os = "macos"))]
        let result = std::process::Command::new("xdg-open").arg(url).status();

        if !matches!(result, Ok(status) if status.success()) {
            eprintln!("Paste into Chrome: {url}");
        }
    }

    let active = AtomicUsize::new(0);
    std::thread::scope(|scope| -> Result<()> {
        for stream in listener.incoming() {
            let stream = stream?;

            if active.load(Ordering::Relaxed) >= 32 {
                continue;
            }

            active.fetch_add(1, Ordering::Relaxed);
            let (options, authority, token, active) = (&options, &authority, &token, &active);
            scope.spawn(move || {
                if let Err(error) = connection(stream, options, authority, token) {
                    eprintln!("CDP connection: {error}");
                }

                active.fetch_sub(1, Ordering::Relaxed);
            });
        }

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_rebinding_and_web_origins() {
        assert!(allowed(Some("127.0.0.1:1234"), None, "127.0.0.1:1234"));
        assert!(allowed(
            Some("127.0.0.1:1234"),
            Some("devtools://devtools"),
            "127.0.0.1:1234"
        ));
        assert!(!allowed(Some("evil.test:1234"), None, "127.0.0.1:1234"));
        assert!(!allowed(
            Some("127.0.0.1:1234"),
            Some("http://localhost"),
            "127.0.0.1:1234"
        ));
        assert!(!allowed(None, None, "127.0.0.1:1234"));
    }
}
