//! Native updater startup; managed packages keep their operating-system owner.

/// Runs provider startup before any app factory, services or worker threads.
///
/// The Velopack SDK may intentionally exit/restart for installer callbacks.
/// Automatic application of downloaded updates is disabled; installation must
/// pass the application's explicit preparation/approval policy.
pub fn early_startup() {
    #[cfg(all(windows, feature = "velopack"))]
    velopack::VelopackApp::build()
        .set_auto_apply_on_startup(false)
        .run();
}
