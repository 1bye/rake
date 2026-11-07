//! Browser panic persistence and Web Locks delivery; no app callback in the hook.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::record::{encode, panic_message, prefix, source_path};
use super::{CrashConfig, CrashRecord, CrashSink};
use crate::diagnostics::Collector;

static NEXT_RECORD: AtomicU32 = AtomicU32::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);

#[wasm_bindgen(module = "/src/crash/browser.js")]
extern "C" {
    #[wasm_bindgen(catch)]
    fn crash_open() -> Result<(), JsValue>;
    fn crash_prune(prefix: &str, count: usize);
    #[wasm_bindgen(catch)]
    fn crash_write(
        prefix: &str,
        id: &str,
        raw: &str,
        bytes: usize,
        count: usize,
    ) -> Result<(), JsValue>;
    fn crash_deliver(
        prefix: &str,
        bytes: usize,
        count: usize,
        report: &js_sys::Function,
    ) -> js_sys::Promise;
}

fn storage_error(_: JsValue) -> io::Error {
    // Browser exception messages can contain host/user data.
    io::Error::other("browser crash storage or delivery unavailable")
}

#[derive(Clone)]
pub(crate) struct CrashReporter {
    prefix: String,
    config: CrashConfig,
}

impl CrashConfig {
    pub(crate) fn open(self, application_id: &str) -> io::Result<CrashReporter> {
        self.validate(application_id)?;
        let prefix = format!("rake:crashes:{application_id}:");
        crash_open().map_err(storage_error)?;
        Ok(CrashReporter {
            prefix,
            config: self,
        })
    }
}

impl CrashReporter {
    pub(crate) fn prune(&self) {
        crash_prune(&self.prefix, self.config.limits.records);
    }

    pub(crate) fn install(&self, collector: Collector) -> io::Result<()> {
        console_error_panic_hook::set_once();
        if INSTALLED.swap(true, Ordering::AcqRel) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "crash hook already installed",
            ));
        }
        // Generate randomness outside the panic hook, separately in each page.
        let page_id = uuid::Uuid::new_v4().as_u128();
        let reporter = self.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let timestamp_ms = js_sys::Date::now().max(0.) as u64;
            let id = format!(
                "{timestamp_ms:020}-{page_id:039}-{:010}",
                NEXT_RECORD.fetch_add(1, Ordering::Relaxed)
            );
            let location = info.location().map(|location| {
                format!(
                    "{}:{}:{}",
                    source_path(location.file()),
                    location.line(),
                    location.column()
                )
            });
            let (message, truncated) =
                panic_message(info.payload(), reporter.config.include_panic_message);
            let mut record = CrashRecord {
                schema_version: 1,
                id,
                timestamp_ms,
                message,
                truncated: truncated
                    || reporter.config.version.len() > 128
                    || location.as_ref().is_some_and(|text| text.len() > 256),
                location: location.map(|text| prefix(&text, 256).into()),
                // No std backtrace on wasm; JS Error stacks may contain private URLs.
                backtrace: String::new(),
                app_version: prefix(&reporter.config.version, 128).into(),
                os: std::env::consts::OS.into(),
                diagnostics: collector.try_crash_tail(reporter.config.limits.diagnostics),
            };
            if let Ok(bytes) = encode(&mut record, reporter.config.limits.record_bytes)
                && let Ok(raw) = std::str::from_utf8(&bytes)
            {
                let _ = crash_write(
                    &reporter.prefix,
                    &record.id,
                    raw,
                    reporter.config.limits.record_bytes,
                    reporter.config.limits.records,
                );
            }
            previous(info);
        }));
        Ok(())
    }

    pub(crate) async fn deliver(&self, sink: Box<dyn CrashSink>) -> io::Result<()> {
        let report = Closure::wrap(Box::new(move |raw: String| -> Result<u32, JsValue> {
            #[derive(serde::Deserialize)]
            struct Version {
                schema_version: u64,
            }
            if serde_json::from_str::<Version>(&raw).is_ok_and(|v| v.schema_version > 1) {
                return Ok(0);
            }
            let record = serde_json::from_str::<CrashRecord>(&raw)
                .ok()
                .filter(|record| {
                    record.schema_version == 1
                        && !record.id.is_empty()
                        && record.id.len() <= 96
                        && record
                            .id
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || byte == b'-')
                });
            let Some(record) = record else {
                return Ok(2);
            };
            sink.report(&record)
                .map_err(|_| JsValue::from_str("crash sink failed; retry later"))?;
            Ok(1)
        }) as Box<dyn FnMut(String) -> Result<u32, JsValue>>);
        JsFuture::from(crash_deliver(
            &self.prefix,
            self.config.limits.record_bytes,
            self.config.limits.records,
            report.as_ref().unchecked_ref(),
        ))
        .await
        .map_err(storage_error)?;
        Ok(())
    }
}
