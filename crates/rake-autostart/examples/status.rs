//! Read-only smoke check; never changes login state.
fn main() -> Result<(), rake_autostart::Error> {
    let login = rake_autostart::LaunchAtLogin::new("dev.rake.tray-example")?;
    println!("{:?}", futures::executor::block_on(login.status())?);
    Ok(())
}
