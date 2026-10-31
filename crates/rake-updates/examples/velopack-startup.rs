//! Native installer startup probe: --veloapp-install 1.0.0 must exit before app construction.
fn main() {
    rake_updates::early_startup_for(Some(r#"{"schema_version":1,"identifier":"dev.rake.startup","version":"1.0.0","package_version":"1.0.0","target":"x86_64-pc-windows-msvc","format":"velopack","owner":"disabled","channel":"stable","base_url":null,"public_keys":{}}"#)).unwrap();
    panic!("installer probe reached the application factory");
}
