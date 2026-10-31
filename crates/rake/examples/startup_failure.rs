//! Intentionally fails root initialization; scripts/test-app-startup.py checks the exit contract.
#[rake::app]
fn application() -> Result<rake::App, std::io::Error> {
    Ok(rake::App::new("example.startup-failure")
        .try_root(|_, _, _| Err(rake::router::MountError("expected startup failure".into()))))
}
