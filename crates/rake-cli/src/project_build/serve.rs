//! Bounded loopback Web serving with a native broker and retained preview transport.

use super::{Result, bundle::insert};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};

pub(super) struct Config {
    pub(super) directory: PathBuf,
    pub(super) development: Option<PathBuf>,
    pub(super) builds: Option<PathBuf>,
    pub(super) control_only: bool,
    pub(super) backend: Option<SocketAddr>,
    pub(super) broker: Option<Arc<super::broker::Broker>>,
}

pub(super) struct Server {
    broker: Option<Arc<super::broker::Broker>>,
    stopping: Arc<AtomicBool>,
    connections: Arc<Mutex<HashMap<u64, TcpStream>>>,
    worker: Option<JoinHandle<std::result::Result<(), String>>>,
}

impl Server {
    pub(super) fn start(listener: TcpListener, config: Config) -> Result<Self> {
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let stopping = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(HashMap::new()));
        let broker = config.broker.clone();
        let stopped = stopping.clone();
        let sockets = connections.clone();
        let worker = thread::spawn(move || {
            thread::scope(|scope| {
                let config = &config;
                let mut next = 0;
                let mut workers = Vec::new();
                while !stopped.load(Ordering::Acquire) {
                    workers.retain_mut(|worker: &mut Option<thread::ScopedJoinHandle<'_, ()>>| {
                        if worker.as_ref().is_some_and(|worker| worker.is_finished()) {
                            let _ = worker.take().expect("finished worker exists").join();
                            false
                        } else {
                            true
                        }
                    });
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let mut sockets_guard =
                                sockets.lock().expect("Web connection lock poisoned");
                            if stopped.load(Ordering::Acquire) || sockets_guard.len() >= 64 {
                                continue;
                            }
                            next += 1;
                            let id = next;
                            sockets_guard
                                .insert(id, stream.try_clone().map_err(|error| error.to_string())?);
                            drop(sockets_guard);
                            let sockets = &sockets;
                            let stopped = &stopped;
                            workers.push(Some(scope.spawn(move || {
                                let _ = handle(stream, address, config, stopped);
                                sockets
                                    .lock()
                                    .expect("Web connection lock poisoned")
                                    .remove(&id);
                            })));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10))
                        }
                        Err(error) => return Err(error.to_string()),
                    }
                }
                Ok(())
            })
        });
        Ok(Self {
            broker,
            stopping,
            connections,
            worker: Some(worker),
        })
    }

    pub(super) fn check(&self) -> Result<()> {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return Err("development Web server exited".into());
        }
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(broker) = &self.broker {
            broker.close();
        }
        for stream in self
            .connections
            .lock()
            .expect("Web connection lock poisoned")
            .values()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}
impl Request {
    fn header(&self, name: &str) -> &str {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .unwrap_or("")
    }
}

fn request(stream: &mut TcpStream) -> Result<Request> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
        if bytes.len() > 64 * 1024 {
            return Err("Web request headers exceed 64 KiB".into());
        }
    }
    // HTTP fields use ISO-8859-1, including invalid/non-ASCII authentication tokens.
    let text: String = bytes.into_iter().map(char::from).collect();
    let mut lines = text.split("\r\n");
    let mut first = lines
        .next()
        .ok_or("missing HTTP request")?
        .split_whitespace();
    let method = first.next().ok_or("missing HTTP method")?.to_owned();
    let path = first.next().ok_or("missing HTTP path")?.to_owned();
    let mut headers = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or("invalid HTTP header")?;
        headers.push((name.to_owned(), value.trim().to_owned()));
    }
    Ok(Request {
        method,
        path,
        headers,
    })
}

fn state(config: &Config) -> Value {
    config
        .development
        .as_ref()
        .and_then(|path| fs::File::open(path).ok())
        .and_then(|file| serde_json::from_reader(file.take(1024 * 1024)).ok())
        .unwrap_or_else(|| serde_json::json!({"published":0,"phase":"starting"}))
}

fn translated(config: &Config, request: &Request) -> Option<PathBuf> {
    let raw = request.path.split(['?', '#']).next()?;
    let decoded = percent_encoding::percent_decode_str(raw).decode_utf8_lossy();
    let prefix = "/__rake/build/";
    if let Some(builds) = &config.builds
        && let Some(relative) = decoded.strip_prefix(prefix)
    {
        let parts: Vec<_> = relative.split('/').collect();
        if parts
            .first()
            .is_none_or(|part| part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()))
            || parts.iter().any(|part| matches!(*part, "." | ".."))
        {
            return None;
        }
        let candidate = builds.join(relative);
        if !super::resolve(&candidate)
            .ok()?
            .starts_with(fs::canonicalize(builds).ok()?)
        {
            return None;
        }
        return Some(candidate);
    }
    let development = state(config);
    let mut active = development["directory"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| config.directory.clone());
    if let Some(builds) = &config.builds
        && let Ok(referer) = url::Url::parse(request.header("Referer"))
    {
        let host = request
            .header("Referer")
            .split_once("://")
            .map(|(_, tail)| tail.split(['/', '?', '#']).next().unwrap_or_default())
            .unwrap_or_default();
        let path = percent_encoding::percent_decode_str(referer.path()).decode_utf8_lossy();
        if host == request.header("Host")
            && let Some(relative) = path.strip_prefix(prefix)
        {
            let build = relative.split('/').next()?;
            if !build.is_empty() && build.bytes().all(|c| c.is_ascii_digit()) {
                active = builds.join(build);
            }
        }
    }
    // SimpleHTTPRequestHandler normalizes dot segments and drops drive/path components.
    let mut parts = Vec::new();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    for part in parts {
        active.push(part);
    }
    Some(active)
}

fn handle(
    mut stream: TcpStream,
    address: SocketAddr,
    config: &Config,
    stopping: &AtomicBool,
) -> Result<()> {
    // macOS accept inherits the listener's O_NONBLOCK; each worker uses blocking I/O.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    let request = match request(&mut stream) {
        Ok(request) => request,
        Err(_) => return Ok(()),
    };
    let control = config.broker.is_some() || config.backend.is_some();
    let valid_host = [address.to_string(), format!("localhost:{}", address.port())]
        .iter()
        .any(|host| host == request.header("Host"));
    if control && !valid_host {
        return error(
            &mut stream,
            &request,
            403,
            if request.method == "POST" {
                "development bridge authentication failed"
            } else {
                "invalid development host"
            },
        );
    }
    let route = request.path.split('?').next().unwrap_or_default();
    if request.method == "POST" {
        if let Some(broker) = &config.broker {
            let expected = format!("http://{}", request.header("Host"));
            if (!request.header("Origin").is_empty() && request.header("Origin") != expected)
                || !broker.authenticate(request.header("X-Rake-Token"))
            {
                return error(
                    &mut stream,
                    &request,
                    403,
                    "development bridge authentication failed",
                );
            }
            // Preview remains a separately owned transport until phase 5.
            if request.path.starts_with("/__rake/preview")
                && let Some(backend) = config.backend
            {
                return proxy(stream, request, backend, stopping);
            }
            return control_request(&mut stream, &request, broker);
        }
        if let Some(backend) = config.backend {
            return proxy(stream, request, backend, stopping);
        }
        return error(
            &mut stream,
            &request,
            403,
            "development bridge authentication failed",
        );
    }
    if route.starts_with("/__rake/preview")
        && let Some(backend) = config.backend
    {
        return proxy(stream, request, backend, stopping);
    }
    if !["GET", "HEAD"].contains(&request.method.as_str()) {
        return error(
            &mut stream,
            &request,
            501,
            &format!("Unsupported method ('{}')", request.method),
        );
    }
    if route.starts_with("/__rake/inspector") || route.starts_with("/__rake/elements") {
        return error(
            &mut stream,
            &request,
            410,
            "Rake Inspector is deprecated; use Chrome DevTools (--cdp)",
        );
    }
    if config.control_only {
        return error(
            &mut stream,
            &request,
            404,
            "native control serves no application assets",
        );
    }
    if control && route == "/__rake/bootstrap.js" {
        return respond(
            &mut stream,
            &request,
            200,
            "text/javascript; charset=utf-8",
            include_bytes!("../../scripts/web-bootstrap.js"),
            &[("Cache-Control", "no-store".into())],
        );
    }
    let mut path = translated(config, &request);
    let shell_fallback = request.header("Accept").contains("text/html")
        && !request.path.starts_with("/__rake/")
        && path.as_ref().is_none_or(|path| !path.is_file());
    if shell_fallback {
        path = translated(
            config,
            &Request {
                method: request.method.clone(),
                path: "/index.html".into(),
                headers: request.headers.clone(),
            },
        );
    }
    let Some(mut path) = path else {
        return error(&mut stream, &request, 404, "File not found");
    };
    if control && (route == "/" || route == "/index.html" || shell_fallback) {
        let development = state(config);
        let active = development["directory"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| config.directory.clone());
        let build = development["published"].as_u64().unwrap_or(0);
        let mut document = fs::read_to_string(active.join("index.html"))?;
        if build > 0 {
            document = regex::Regex::new(r#"(['"])(?:/|\./)?pkg/"#)?
                .replace_all(&document, |capture: &regex::Captures<'_>| {
                    format!("{}/__rake/build/{build}/pkg/", &capture[1])
                })
                .into_owned();
            document = regex::Regex::new(r#"(src|href)=(["'])/"#)?
                .replace_all(&document, |capture: &regex::Captures<'_>| {
                    if document[capture.get(0).expect("whole match").end()..].starts_with("__rake/")
                    {
                        capture[0].to_owned()
                    } else {
                        format!("{}={}/__rake/build/{build}/", &capture[1], &capture[2])
                    }
                })
                .into_owned();
        }
        let base = if build > 0 {
            format!("<base href=\"/__rake/build/{build}/\">")
        } else {
            String::new()
        };
        let query = if config.development.is_some() {
            format!("?build={build}")
        } else {
            String::new()
        };
        document = insert(
            &document,
            &format!("{base}<script src=\"/__rake/bootstrap.js{query}\"></script>"),
        )?;
        return respond(
            &mut stream,
            &request,
            200,
            "text/html; charset=utf-8",
            document.as_bytes(),
            &[("Cache-Control", "no-store".into())],
        );
    }
    if path.is_dir() {
        if !route.ends_with('/') {
            let location = match request.path.split_once('?') {
                Some((path, query)) => format!("{path}/?{query}"),
                None => format!("{}/", request.path),
            };
            return respond(
                &mut stream,
                &request,
                301,
                "text/html",
                &[],
                &[("Location", location)],
            );
        }
        if path.join("index.html").is_file() {
            path.push("index.html");
        } else if path.join("index.htm").is_file() {
            path.push("index.htm");
        } else {
            return listing(&mut stream, &request, &path);
        }
    }
    let Ok(mut file) = fs::File::open(&path) else {
        return error(&mut stream, &request, 404, "File not found");
    };
    let metadata = file.metadata()?;
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    if request.header("If-None-Match").is_empty()
        && let Ok(since) = httpdate::parse_http_date(request.header("If-Modified-Since"))
        && modified.duration_since(SystemTime::UNIX_EPOCH)?.as_secs()
            <= since.duration_since(SystemTime::UNIX_EPOCH)?.as_secs()
    {
        return respond(&mut stream, &request, 304, "text/plain", &[], &[]);
    }
    let mime = if path
        .extension()
        .is_some_and(|ext| ext == "js" || ext == "mjs")
    {
        "text/javascript".into()
    } else {
        mime_guess::from_path(&path)
            .first_or_octet_stream()
            .to_string()
    };
    headers(
        &mut stream,
        200,
        &mime,
        metadata.len(),
        &[("Last-Modified", httpdate::fmt_http_date(modified))],
    )?;
    if request.method != "HEAD" {
        std::io::copy(&mut file, &mut stream)?;
    }
    Ok(())
}

fn control_request(
    stream: &mut TcpStream,
    request: &Request,
    broker: &super::broker::Broker,
) -> Result<()> {
    use super::broker::{Error, MAX_BYTES};
    let mut consumed = false;
    let result = (|| -> std::result::Result<Vec<u8>, Error> {
        let length = request
            .header("Content-Length")
            .parse::<usize>()
            .ok()
            .filter(|length| *length <= MAX_BYTES - 1024);
        if length.is_none() || request.header("Content-Type") != "application/json" {
            return Err(Error::new(413, "invalid or oversized JSON frame"));
        }
        let mut bytes = vec![0; length.expect("validated length")];
        stream.read_exact(&mut bytes)?;
        consumed = true;
        let body: Value = serde_json::from_slice(&bytes)?;
        if !body.is_object() {
            return Err(Error::new(400, "expected JSON object"));
        }
        let action = request
            .path
            .strip_prefix("/__rake/")
            .ok_or_else(|| Error::new(404, "unknown development endpoint"))?;
        let result = broker.handle(action, &body)?;
        let encoded = serde_json::to_vec(&result)?;
        if encoded.len() > MAX_BYTES {
            return Err(Error::new(413, "development response too large"));
        }
        Ok(encoded)
    })();
    match result {
        Ok(encoded) => respond(
            stream,
            request,
            200,
            "application/json",
            &encoded,
            &[("Cache-Control", "no-store".into())],
        ),
        // The body was already consumed, so do not drain it again on an error.
        Err(failure) => {
            let consumed_request = Request {
                method: request.method.clone(),
                path: request.path.clone(),
                headers: Vec::new(),
            };
            error(
                stream,
                if consumed { &consumed_request } else { request },
                failure.status,
                &failure.to_string(),
            )
        }
    }
}

fn proxy(
    mut client: TcpStream,
    request: Request,
    address: SocketAddr,
    stopping: &AtomicBool,
) -> Result<()> {
    let expected = format!("http://{}", request.header("Host"));
    if !request.header("Origin").is_empty() && request.header("Origin") != expected {
        return error(
            &mut client,
            &request,
            403,
            "development bridge authentication failed",
        );
    }
    let mut backend = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    let mut frame = format!("{} {} HTTP/1.1\r\n", request.method, request.path).into_bytes();
    for (name, value) in &request.headers {
        let value = if name.eq_ignore_ascii_case("Host") {
            address.to_string()
        } else if name.eq_ignore_ascii_case("Origin") {
            format!("http://{address}")
        } else {
            value.clone()
        };
        frame.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    frame.extend_from_slice(b"\r\n");
    // Control requests are bounded JSON frames; deliver their body with the headers
    // so an early backend authentication error cannot race a later body write.
    if request.method == "POST"
        && let Ok(length) = request.header("Content-Length").parse::<usize>()
        && length <= 1024 * 1024
    {
        let end = frame.len();
        frame.resize(end + length, 0);
        client.read_exact(&mut frame[end..])?;
    }
    backend.write_all(&frame)?;
    let mut inbound = client.try_clone()?;
    let mut outbound = backend.try_clone()?;
    let finished = AtomicBool::new(false);
    thread::scope(|scope| {
        let finished = &finished;
        let worker = scope.spawn(move || {
            let _ = transfer(&mut inbound, &mut outbound, stopping, finished);
        });
        let result = transfer(&mut backend, &mut client, stopping, finished);
        finished.store(true, Ordering::Release);
        let _ = backend.shutdown(Shutdown::Both);
        let _ = client.shutdown(Shutdown::Both);
        let _ = worker.join();
        result
    })
}

fn transfer(
    source: &mut TcpStream,
    destination: &mut TcpStream,
    stopping: &AtomicBool,
    finished: &AtomicBool,
) -> Result<()> {
    source.set_read_timeout(Some(Duration::from_millis(200)))?;
    destination.set_write_timeout(Some(Duration::from_secs(15)))?;
    let mut buffer = [0; 8192];
    let mut last = Instant::now();
    while !stopping.load(Ordering::Acquire)
        && !finished.load(Ordering::Acquire)
        && last.elapsed() < Duration::from_secs(15)
    {
        match source.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                destination.write_all(&buffer[..count])?;
                last = Instant::now();
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        301 => "Moved Permanently",
        304 => "Not Modified",
        400 => "Bad Request",
        403 => "Forbidden",
        413 => "Request Entity Too Large",
        429 => "Too Many Requests",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        404 => "Not Found",
        410 => "Gone",
        501 => "Not Implemented",
        _ => "Error",
    }
}
fn headers(
    stream: &mut TcpStream,
    status: u16,
    mime: &str,
    length: u64,
    extra: &[(&str, String)],
) -> Result<()> {
    write!(
        stream,
        "HTTP/1.0 {status} {}\r\nServer: rake\r\nDate: {}\r\nContent-Type: {mime}\r\nContent-Length: {length}\r\n",
        reason(status),
        httpdate::fmt_http_date(SystemTime::now())
    )?;
    for (name, value) in extra {
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(stream, "\r\n")?;
    Ok(())
}
fn respond(
    stream: &mut TcpStream,
    request: &Request,
    status: u16,
    mime: &str,
    body: &[u8],
    extra: &[(&str, String)],
) -> Result<()> {
    headers(stream, status, mime, body.len() as u64, extra)?;
    if request.method != "HEAD" {
        stream.write_all(body)?;
    }
    Ok(())
}
fn error(stream: &mut TcpStream, request: &Request, status: u16, message: &str) -> Result<()> {
    // Closing a socket with unread POST bytes can reset it before the error reaches the client.
    if let Ok(length) = request.header("Content-Length").parse::<u64>() {
        stream.set_read_timeout(Some(Duration::from_millis(200)))?;
        let _ = std::io::copy(
            &mut (&mut *stream).take(length.min(1024 * 1024)),
            &mut std::io::sink(),
        );
    }

    let explanation = match status {
        403 => "Request forbidden -- authorization will not help",
        404 => "Nothing matches the given URI",
        410 => "URI no longer exists and has been permanently removed",
        501 => "Server does not support this operation",
        _ => "",
    };
    let body = format!(
        "<!DOCTYPE HTML>\n<html lang=\"en\">\n    <head>\n        <meta charset=\"utf-8\">\n        <title>Error response</title>\n    </head>\n    <body>\n        <h1>Error response</h1>\n        <p>Error code: {status}</p>\n        <p>Message: {}.</p>\n        <p>Error code explanation: {status} - {explanation}.</p>\n    </body>\n</html>\n",
        escape_text(message)
    );
    respond(
        stream,
        request,
        status,
        "text/html;charset=utf-8",
        body.as_bytes(),
        &[("Connection", "close".into())],
    )
}
fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn listing(stream: &mut TcpStream, request: &Request, path: &Path) -> Result<()> {
    let mut entries = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
    let display = super::bundle::html(
        &percent_encoding::percent_decode_str(&request.path).decode_utf8_lossy(),
    );
    let mut body = format!(
        "<!DOCTYPE HTML>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Directory listing for {display}</title>\n</head>\n<body>\n<h1>Directory listing for {display}</h1>\n<hr>\n<ul>\n"
    );
    for entry in entries {
        let mut name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() {
            name.push('/');
        }
        let encoded = name
            .bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                    (byte as char).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect::<String>();
        body.push_str(&format!(
            "<li><a href=\"{encoded}\">{}</a></li>\n",
            escape_text(&name)
        ));
    }
    body.push_str("</ul>\n<hr>\n</body>\n</html>\n");
    respond(
        stream,
        request,
        200,
        "text/html; charset=utf-8",
        body.as_bytes(),
        &[],
    )
}

#[cfg(test)]
mod tests {
    //! Covers: static MIME/HEAD/cache, app-shell rewrites, immutable builds, confinement and shutdown.
    //! Real: native server, HTTP sockets and filesystem. Doubles: build/state files.
    //! Requires: loopback sockets.
    use super::*;

    fn get(address: SocketAddr, method: &str, path: &str, headers: &str) -> (u16, String, Vec<u8>) {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {address}\r\n{headers}\r\n"
        )
        .unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).unwrap();
        let end = reply
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = String::from_utf8(reply[..end].to_vec()).unwrap();
        (
            headers.split_whitespace().nth(1).unwrap().parse().unwrap(),
            headers,
            reply[end..].to_vec(),
        )
    }
    #[test]
    fn static_serving_keeps_mime_head_cache_redirects_and_html_fallback() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("index.html"),
            "<html><head></head>shell</html>",
        )
        .unwrap();
        fs::write(temp.path().join("app.wasm"), b"\0asm").unwrap();
        fs::create_dir(temp.path().join("folder")).unwrap();
        fs::write(temp.path().join("folder/message.txt"), "message").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = Server::start(
            listener,
            Config {
                directory: temp.path().into(),
                development: None,
                builds: None,
                control_only: false,
                backend: None,
                broker: None,
            },
        )
        .unwrap();
        let (status, headers, body) = get(address, "GET", "/app.wasm", "");
        assert_eq!(status, 200);
        assert!(headers.contains("application/wasm"));
        assert_eq!(body, b"\0asm");
        let (_, headers, body) = get(address, "HEAD", "/app.wasm", "");
        assert!(headers.contains("Content-Length: 4"));
        assert!(body.is_empty());
        let modified = headers
            .lines()
            .find_map(|line| line.strip_prefix("Last-Modified: "))
            .unwrap();
        assert_eq!(
            get(
                address,
                "GET",
                "/app.wasm",
                &format!("If-Modified-Since: {modified}\r\n")
            )
            .0,
            304
        );
        assert_eq!(get(address, "GET", "/missing", "").0, 404);
        assert_eq!(
            get(address, "GET", "/route", "Accept: text/html\r\n").2,
            b"<html><head></head>shell</html>"
        );
        let (status, headers, _) = get(address, "GET", "/folder?query=1", "");
        assert_eq!(status, 301);
        assert!(headers.contains("Location: /folder/?query=1"));
        assert!(
            String::from_utf8(get(address, "GET", "/folder/", "").2)
                .unwrap()
                .contains("message.txt")
        );
        // A new connect can hit another parallel test after ephemeral-port reuse.
        let mut pending = TcpStream::connect(address).unwrap();
        pending
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(pending, "GET / HTTP/1.1\r\n").unwrap();
        drop(server);
        let result = pending.read(&mut [0]);
        assert!(
            matches!(result, Ok(0))
                || result.as_ref().is_err_and(|error| matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                )),
            "owned connection must close: {result:?}"
        );
    }
    #[test]
    fn immutable_builds_survive_publication_and_referer_pins_absolute_imports() {
        let temp = tempfile::tempdir().unwrap();
        let builds = temp.path().join("builds");
        for build in [1, 2] {
            fs::create_dir_all(builds.join(format!("{build}/pkg"))).unwrap();
            fs::write(
                builds.join(format!("{build}/pkg/app.js")),
                format!("build {build}"),
            )
            .unwrap();
        }
        let status = temp.path().join("status.json");
        fs::write(
            &status,
            serde_json::json!({"published":2,"directory":builds.join("2")}).to_string(),
        )
        .unwrap();
        let request = |path: &str, referer: String| Request {
            method: "GET".into(),
            path: path.into(),
            headers: vec![
                ("Host".into(), "127.0.0.1:80".into()),
                ("Referer".into(), referer),
            ],
        };
        let config = Config {
            directory: temp.path().into(),
            development: Some(status),
            builds: Some(builds.clone()),
            control_only: false,
            backend: None,
            broker: None,
        };
        assert_eq!(
            translated(
                &config,
                &request("/__rake/build/1/pkg/app.js", String::new())
            )
            .unwrap(),
            builds.join("1/pkg/app.js")
        );
        assert_eq!(
            translated(
                &config,
                &request(
                    "/pkg/app.js",
                    "http://127.0.0.1:80/__rake/build/1/loader.js".into()
                )
            )
            .unwrap(),
            builds.join("1/pkg/app.js")
        );
        assert_eq!(
            translated(
                &config,
                &request(
                    "/pkg/app.js",
                    "http://foreign/__rake/build/1/loader.js".into()
                )
            )
            .unwrap(),
            builds.join("2/pkg/app.js")
        );
        assert!(
            translated(
                &config,
                &request("/__rake/build/1/%2e%2e/status.json", String::new())
            )
            .is_none()
        );
        std::os::unix::fs::symlink(temp.path(), builds.join("1/escape")).unwrap();
        assert!(
            translated(
                &config,
                &request("/__rake/build/1/escape/status.json", String::new())
            )
            .is_none()
        );
    }
    #[test]
    fn shutdown_releases_partial_request_readers_and_checks_control_host() {
        let temp = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = Server::start(
            listener,
            Config {
                directory: temp.path().into(),
                development: None,
                builds: None,
                control_only: false,
                backend: Some("127.0.0.1:9".parse().unwrap()),
                broker: None,
            },
        )
        .unwrap();
        let mut pending = TcpStream::connect(address).unwrap();
        write!(pending, "GET / HTTP/1.1\r\n").unwrap();
        let mut foreign = TcpStream::connect(address).unwrap();
        write!(foreign, "GET / HTTP/1.1\r\nHost: foreign\r\n\r\n").unwrap();
        let mut reply = String::new();
        foreign.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.0 403"));
        let started = Instant::now();
        drop(server);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn rejected_post_bodies_preserve_the_http_error_instead_of_resetting_the_connection() {
        let temp = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let _server = Server::start(
            listener,
            Config {
                directory: temp.path().into(),
                development: None,
                builds: None,
                control_only: false,
                backend: Some("127.0.0.1:9".parse().unwrap()),
                broker: None,
            },
        )
        .unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(stream, "POST /__rake/development HTTP/1.1\r\nHost: {address}\r\nOrigin: http://foreign\r\nContent-Length: 2\r\n\r\n{{}}").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.0 403"), "{response}");
        assert!(response.contains("development bridge authentication failed"));
    }
    #[test]
    fn control_requests_wait_for_segmented_headers_and_json_bodies() {
        let root = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let broker = Arc::new(
            super::super::broker::Broker::new(super::super::broker::Options {
                url: format!("http://{address}"),
                directory: root.path().join("control"),
                development: None,
                output: None,
                allow_native: false,
                cdp_url: None,
                source_root: root.path().into(),
                cli: "/unused".into(),
            })
            .unwrap(),
        );
        let token = broker.token().to_owned();
        let _server = Server::start(
            listener,
            Config {
                directory: root.path().into(),
                development: None,
                builds: None,
                control_only: true,
                backend: None,
                broker: Some(broker),
            },
        )
        .unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        write!(stream, "POST /__rake/development HTTP/1.1\r\n").unwrap();
        let pending = stream.read(&mut [0]);
        assert!(
            pending.is_err_and(|error| matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            )),
            "partial headers must remain pending"
        );
        write!(stream,"Host: {address}\r\nX-Rake-Token: {token}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{").unwrap();
        let pending = stream.read(&mut [0]);
        assert!(
            pending.is_err_and(|error| matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            )),
            "partial JSON must remain pending"
        );
        stream.write_all(b"}").unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.0 200"), "{response}");
        let body: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["published"], 0);
        assert_eq!(body["runtimes"], serde_json::json!({}));
    }
}
