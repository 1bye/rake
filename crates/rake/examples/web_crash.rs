//! Browser regression app: real App builder, panic hook, storage and Rust sink.
//! Run with scripts/test-web-crash.mjs; no window or GPU drawing is exercised.

#[cfg(target_family = "wasm")]
mod browser {
    use rake::crash::{CrashLimits, CrashRecord};
    use rake::gpui::AppContext as _;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(
        inline_js = "export function attempted() { window.sinkAttempts = (window.sinkAttempts || 0) + 1; } export function accepted(raw) { const records = JSON.parse(localStorage.getItem('receipts') || '[]'); records.push(JSON.parse(raw)); localStorage.setItem('receipts', JSON.stringify(records)); }"
    )]
    extern "C" {
        fn accepted(raw: &str);
        fn attempted();
    }

    /// Starts the real background App with an automatic, disabled or explicit policy.
    #[wasm_bindgen]
    pub fn start(mode: &str, include_message: bool, record_bytes: usize) -> Result<(), JsValue> {
        let config = rake::crash_config!()
            .include_panic_message(include_message)
            .limits(CrashLimits {
                records: 2,
                record_bytes,
                diagnostics: 32,
            });
        let mut app = rake::App::new("test.web-crash")
            .distribution_policy(Some(r#"{"version":"consumer-web-1.2.3"}"#), "");
        if mode != "automatic" {
            let fail = mode == "fail";
            let unwind = mode == "unwind";
            app = app.crash_reports(config, move |record: &CrashRecord| {
                attempted();
                assert!(!unwind, "sink panic");
                if fail {
                    return Err(std::io::Error::other("retry"));
                }
                accepted(&rake::serde_json::to_string(record)?);
                Ok(())
            });
        }
        if mode == "disabled" {
            app = app.without_crash_reports();
        }
        app.root(|_, _, cx| cx.new(|_| rake::gpui::Empty).into())
            .run()
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Triggers a real Rust panic; the next page load must deliver its record.
    #[wasm_bindgen]
    pub fn panic_now(large: bool) {
        if large {
            panic!("{}", "🦀\\\"\nSECRET_USER_CONTENT".repeat(10_000));
        }
        panic!("SECRET_USER_CONTENT");
    }
}
