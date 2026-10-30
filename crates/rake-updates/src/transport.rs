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
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(5)
            .timeout_global(Some(Duration::from_secs(300)))
            .timeout_per_read(Some(Duration::from_secs(15)))
            .timeout_connect(Some(Duration::from_secs(15)))
            .build(),
    )
}
pub(crate) fn get(url: &str, bound: u64, cancelled: &AtomicBool) -> Result<Vec<u8>, Error> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let mut response = agent()
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
    url: &str,
    total: u64,
    output: &mut impl Write,
    cancelled: &AtomicBool,
    progress: &mut impl FnMut(DownloadProgress),
) -> Result<(), Error> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let mut response = agent()
        .get(url)
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
