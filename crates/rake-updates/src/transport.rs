//! Bounded HTTPS transport shared by native providers.
use crate::{DownloadProgress, Error};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
pub(crate) fn agent() -> ureq::Agent {
    configured_agent(None)
}
pub(crate) fn configured_agent(root: Option<ureq::tls::Certificate<'static>>) -> ureq::Agent {
    let tls = if let Some(root) = root {
        ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::Specific(std::sync::Arc::new(vec![
                root,
            ])))
            .build()
    } else {
        ureq::tls::TlsConfig::default()
    };
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(true)
            .max_redirects(5)
            .timeout_global(Some(Duration::from_secs(300)))
            .timeout_per_read(Some(Duration::from_secs(15)))
            .timeout_connect(Some(Duration::from_secs(15)))
            .build(),
    )
}
pub(crate) fn get(
    agent: &ureq::Agent,
    url: &str,
    bound: u64,
    cancelled: &AtomicBool,
) -> Result<Vec<u8>, Error> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| Error::Provider(error.to_string()))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(bound + 1)
        .read_to_end(&mut bytes)?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    if bytes.len() as u64 > bound {
        return Err(Error::Provider("feed exceeds download bound".into()));
    }
    Ok(bytes)
}
pub(crate) fn download(
    agent: &ureq::Agent,
    url: &str,
    total: u64,
    output: &mut impl Write,
    cancelled: &AtomicBool,
    progress: &mut impl FnMut(DownloadProgress),
) -> Result<(), Error> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let mut response = agent
        .get(url)
        .config()
        // Package size is bounded, but a progressing download has no total deadline.
        .timeout_global(None)
        .timeout_recv_response(Some(Duration::from_secs(15)))
        .build()
        .call()
        .map_err(|error| Error::Provider(error.to_string()))?;
    let mut reader = response.body_mut().as_reader();
    let mut buffer = [0; 64 * 1024];
    let mut received = 0;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > total {
            return Err(Error::Provider("image exceeds signed length".into()));
        }
        output.write_all(&buffer[..count])?;
        progress(DownloadProgress {
            total_bytes: total,
            received_bytes: received,
        });
    }
    if received != total {
        return Err(Error::Provider("truncated image download".into()));
    }
    Ok(())
}
pub(crate) fn digest(path: &Path) -> Result<String, Error> {
    let mut hash = Sha256::new();
    std::io::copy(&mut fs::File::open(path)?, &mut hash)?;
    Ok(hex::encode(hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn progressing_download_outlives_global_budget_and_can_cancel() {
        for cancel in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let (resume, resumed) = mpsc::channel();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = BufReader::new(&stream);
                let mut line = String::new();

                loop {
                    line.clear();
                    request.read_line(&mut line).unwrap();

                    if line == "\r\n" {
                        break;
                    }
                }

                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\na")
                    .unwrap();
                resumed.recv_timeout(Duration::from_secs(5)).unwrap();
                let _ = stream.write_all(b"b");
            });
            let agent = ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_millis(50)))
                    .timeout_per_read(Some(Duration::from_secs(2)))
                    .build(),
            );
            let cancelled = AtomicBool::new(false);
            let mut bytes = Vec::new();
            let result = download(&agent, &url, 2, &mut bytes, &cancelled, &mut |progress| {
                if progress.received_bytes == 1 {
                    std::thread::sleep(Duration::from_millis(100));
                    cancelled.store(cancel, Ordering::Relaxed);
                    resume.send(()).unwrap();
                }
            });

            server.join().unwrap();

            if cancel {
                assert!(matches!(result, Err(Error::Cancelled)));
                assert_eq!(bytes, b"a");
            } else {
                result.unwrap();
                assert_eq!(bytes, b"ab");
            }
        }
    }

    #[test]
    fn stalled_download_still_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (finish, finished) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = BufReader::new(&stream);
            let mut line = String::new();

            loop {
                line.clear();
                request.read_line(&mut line).unwrap();

                if line == "\r\n" {
                    break;
                }
            }

            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
                .unwrap();
            finished.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .timeout_per_read(Some(Duration::from_secs(1)))
                .build(),
        );
        let result = download(
            &agent,
            &url,
            1,
            &mut Vec::new(),
            &AtomicBool::new(false),
            &mut |_| {},
        );

        finish.send(()).unwrap();
        server.join().unwrap();

        assert!(matches!(result, Err(Error::Io(_))));
    }

    #[test]
    fn metadata_and_stall_timeouts_remain_bounded() {
        let agent = agent();
        let config = agent.config();

        assert_eq!(config.timeouts().global, Some(Duration::from_secs(300)));
        assert_eq!(config.timeouts().connect, Some(Duration::from_secs(15)));
        assert_eq!(config.timeout_per_read(), Some(Duration::from_secs(15)));
    }
}
