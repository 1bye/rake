//! Native installer startup probe: --veloapp-version must exit before app construction.
fn main() {
    rake_updates::early_startup();
    panic!("installer probe reached the application factory");
}
