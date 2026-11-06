//! Covers: one shared native/Web App builder and Web's immediate sink release.
//! Real: App and crash public API; no hook installation or native launch.
//! Doubles: sink whose callback must never run during configuration.
//! Requires: gpui feature; wasm check compiles this test without running it.

#![cfg(feature = "gpui")]

use std::cell::Cell;
use std::rc::Rc;

use rake::crash::{CrashLimits, CrashRecord, CrashSink};

struct Sink(Rc<Cell<usize>>);

impl CrashSink for Sink {
    fn report(&self, _: &CrashRecord) -> std::io::Result<()> {
        panic!("configuring crash reports must not deliver records")
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn shared_builder_accepts_crash_reports_without_launching() {
    let dropped = Rc::new(Cell::new(0));
    let app = rake::App::new("test.crash").crash_reports(
        rake::crash_config!()
            .directory("unused-crash-directory")
            .limits(CrashLimits::default()),
        Sink(dropped.clone()),
    );

    assert_eq!(dropped.get(), usize::from(cfg!(target_family = "wasm")));
    drop(app);
    assert_eq!(dropped.get(), 1);
}

#[test]
fn crash_builder_last_call_wins_and_opt_out_releases_sink() {
    let dropped = Rc::new(Cell::new(0));
    let app = rake::App::new("test.crash")
        .crash_reports(rake::crash_config!(), Sink(dropped.clone()))
        .without_crash_reports();

    assert_eq!(dropped.get(), 1);
    drop(app);
    assert_eq!(dropped.get(), 1);
    let dropped = Rc::new(Cell::new(0));
    let app = rake::App::new("test.crash")
        .without_crash_reports()
        .crash_reports(rake::crash_config!(), Sink(dropped.clone()));

    assert_eq!(dropped.get(), usize::from(cfg!(target_family = "wasm")));
    drop(app);
    assert_eq!(dropped.get(), 1);
}
