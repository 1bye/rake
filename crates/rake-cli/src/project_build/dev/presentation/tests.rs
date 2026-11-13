//! Covers: startup, confirmed live outcomes, failures, reset warnings and terminal-width safety.
//! Real: presentation state and cross-platform termcolor buffers. Doubles: events and clock instants.
//! Requires: none; no terminal, compiler, GPU, process or network.

use super::*;
use serde_json::json;

fn display(web: bool, live: bool) -> Presentation {
    let args = Args {
        web,
        compiler: if live {
            Compiler::Blaze
        } else {
            Compiler::Cargo
        },
        ..Args::default()
    };
    let shared = Presentation::new(&args);
    let mut display = Arc::try_unwrap(shared)
        .ok()
        .expect("unique presentation")
        .into_inner()
        .unwrap();
    display.buffer = Buffer::no_color();
    display.interactive = false;
    display.unicode = true;
    display.name = "Fieldnotes".into();

    display
}

fn render(display: &mut Presentation, event: Value, available: bool, elapsed_ms: u64) -> String {
    display.buffer.clear();
    display
        .render(
            &event,
            available,
            display.started + Duration::from_millis(elapsed_ms),
        )
        .unwrap();

    String::from_utf8(display.buffer.as_slice().to_vec()).unwrap()
}

#[test]
fn startup_matches_the_focused_layout_without_internal_events() {
    let mut display = display(false, true);
    assert!(render(&mut display, json!({"event":"starting"}), false, 0).is_empty());
    assert!(
        render(
            &mut display,
            json!({"event":"session","session":"/private/session"}),
            false,
            0
        )
        .is_empty()
    );

    let output = render(&mut display, json!({"event":"running"}), true, 2400);
    assert_eq!(
        output,
        format!(
            "\n  rake v{}  Fieldnotes\n        Desktop app started · 2.4 s\n        Live updates · experimental\n\n        Ctrl+C to stop\n\n",
            env!("CARGO_PKG_VERSION")
        )
    );
    assert!(!output.contains("Blaze"));
    assert!(!output.contains("session"));
    assert!(!output.contains('\x1b'));
    assert!(
        render(
            &mut display,
            json!({"event":"watching","runtime_id":"internal"}),
            true,
            2400
        )
        .is_empty()
    );
}

#[test]
fn compiled_live_edits_need_acknowledgment_and_report_only_once() {
    let mut display = display(false, true);
    display.header = true;
    display.pending = true;
    display.paths = "src/editor.rs".into();
    display.build_started = display.started;

    let ready = render(
        &mut display,
        json!({"event":"watching","outcome":"unchanged"}),
        true,
        100,
    );
    assert!(ready.contains("Update ready"));
    assert!(!ready.contains("Updated"));
    assert!(display.pending);

    let applied = render(
        &mut display,
        json!({"event":"live_patch_applied","sequence":12}),
        true,
        112,
    );
    assert!(applied.contains("✓ Updated  src/editor.rs"), "{applied}");
    assert!(applied.ends_with("112 ms\n"));
    assert!(!display.pending);
    assert!(
        render(
            &mut display,
            json!({"event":"watching","outcome":"applied"}),
            true,
            113
        )
        .is_empty()
    );
}

#[test]
fn web_publishes_a_build_without_claiming_browser_confirmation() {
    let mut display = display(true, false);
    let listening = json!({"event":"web_listening", "url":"http://localhost:8081/#private"});
    assert!(render(&mut display, listening, false, 0).is_empty());

    let started = render(
        &mut display,
        json!({"event":"watching","published":1}),
        true,
        2400,
    );
    assert!(started.contains("Web app serving · 2.4 s"));
    assert!(started.contains("Open http://localhost:8081/#private"));
    assert!(!started.contains("Desktop"));

    display.pending = true;
    let update = render(
        &mut display,
        json!({"event":"watching","published":2}),
        true,
        2500,
    );
    assert!(update.contains("Update ready"));
    assert!(!update.contains("Updated"));
}

#[test]
fn restarts_warn_beforehand_and_do_not_promise_all_state_is_preserved() {
    let mut display = display(false, true);
    display.header = true;
    display.pending = true;
    display.reset_views(&["Draft (entry-7)".into()]);

    let warning = render(
        &mut display,
        json!({"event":"restarted","message":"internal notice"}),
        true,
        100,
    );
    assert!(warning.contains("Draft will reset. No reload state is registered."));
    assert!(!warning.contains("entry-7"));
    let restarted = render(&mut display, json!({"event":"running"}), true, 642);
    assert!(restarted.contains("Restarted"));
    assert!(!restarted.contains("rake v"));
    assert!(!restarted.contains("preserved"));
}

#[test]
fn errors_are_buffered_once_and_progress_never_enters_the_buffer() {
    let mut display = display(false, false);
    display.build_output("   Compiling app v1.0.0\n").unwrap();
    display
        .build_output("    Finished `dev` profile\n")
        .unwrap();
    assert!(display.diagnostics.is_empty());
    display
        .build_output("error[E0308]: mismatched types\n  --> src/editor.rs:84:18\n")
        .unwrap();
    display
        .build_output("error: could not compile app due to 1 previous error\n")
        .unwrap();
    display
        .build_output("rake worker: distribution: Cargo build failed\n")
        .unwrap();
    display
        .build_output("{\"error\":{\"message\":\"distribution: worker exited exit status: 1\"}}\n")
        .unwrap();
    assert_eq!(display.diagnostics.matches("error[E0308]").count(), 1);
    assert!(!display.diagnostics.contains("worker"));
    assert!(!display.diagnostics.contains("could not compile"));

    let failure = render(
        &mut display,
        json!({"event":"build_failed","error":"duplicate compiler tail"}),
        false,
        189,
    );
    assert!(failure.contains("Update failed"));
    assert!(!failure.contains("duplicate"));
    assert!(!failure.contains("Previous version"));
    display.diagnostics.clear();
}

#[test]
fn paused_and_declined_updates_keep_the_previous_app_explicit() {
    let mut display = display(false, true);
    display.header = true;
    display.pending = true;
    let declined = render(
        &mut display,
        json!({"event":"live_patch_declined","reason":"unsupported change"}),
        true,
        100,
    );
    assert!(declined.contains("Update not applied"));
    assert!(declined.contains("unsupported change"));
    assert!(declined.contains("Previous version is still running."));
    assert!(!declined.contains("Updated"));
    let paused = render(
        &mut display,
        json!({"event":"reload_blocked","error":"missing reload state"}),
        false,
        101,
    );
    assert!(paused.contains("Update paused"));
    assert!(!paused.contains("Previous version"));
    assert!(
        render(
            &mut display,
            json!({"event":"reload_blocked","error":"missing reload state"}),
            false,
            102
        )
        .is_empty()
    );
}

#[test]
fn unicode_paths_are_bounded_by_display_columns_and_plain_output_uses_ascii_markers() {
    for width in 0..40 {
        let text = truncate("src/漢字/very-long-editor.rs", width);
        assert!(
            UnicodeWidthStr::width(text.as_str()) <= width,
            "width {width}: {text}"
        );
    }
    assert_eq!(clean("safe\n\x1b[31m\rname"), "safe[31mname");

    let mut display = display(false, false);
    display.unicode = false;
    display.header = true;
    display.pending = true;
    let plain = render(&mut display, json!({"event":"running"}), true, 642);
    assert!(plain.contains("ok Restarted"));
    assert!(!plain.contains(['\x1b', '\r']));
}

#[test]
fn abnormal_diagnostics_are_bounded_without_splitting_utf8() {
    let mut display = display(false, false);
    let text = "漢".repeat(400_000);
    display.build_output(&text).unwrap();
    assert!(display.diagnostics.len() <= 1024 * 1024);
    assert!(display.diagnostics.ends_with('漢'));
    assert!(display.diagnostics_truncated);
    display.diagnostics.clear();
}

#[test]
fn structured_worker_errors_are_readable_when_there_is_no_compiler_diagnostic() {
    let mut display = display(false, false);
    display
        .build_output("{\"error\":{\"message\":\"permission denied: app icon\"}}\n")
        .unwrap();
    assert_eq!(display.diagnostics, "permission denied: app icon\n");
    let failed = render(
        &mut display,
        json!({"event":"build_failed","error":"internal worker failed"}),
        false,
        10,
    );
    assert!(failed.contains("Update failed"));
    assert!(!failed.contains("internal worker"));
    display.diagnostics.clear();
}
