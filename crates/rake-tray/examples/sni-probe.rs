//! Real isolated D-Bus transport; fixture watcher/consumer, no desktop presentation claim.
#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    futures::executor::block_on(rake_tray::native_test::sni())?;
    println!(
        "SNI protocol probe passed: two exports, ARGB, DBusMenu selection, update, watcher loss/recovery, shutdown"
    );
    Ok(())
}
#[cfg(not(unix))]
fn main() {
    eprintln!("SNI probe requires a Unix session bus");
    std::process::exit(1);
}
