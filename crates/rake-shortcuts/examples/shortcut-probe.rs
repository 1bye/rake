//! Short-lived real registration/conflict/cleanup probe; never simulates key presses.
use rake_shortcuts::{Error, Shortcut};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    futures::executor::block_on(async {
        let key = "Ctrl+Alt+Shift+9";
        let mut first = Shortcut::register(key).await?;
        assert!(matches!(
            Shortcut::register(key).await,
            Err(Error::Conflict)
        ));
        first.unregister().await?;
        first.unregister().await?;
        let second = Shortcut::register(key).await?;
        drop(second);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut third = loop {
            match Shortcut::register(key).await {
                Ok(shortcut) => break shortcut,
                Err(Error::Conflict) if std::time::Instant::now() < deadline => {
                    std::thread::yield_now();
                }
                Err(error) => return Err(error),
            }
        };
        third.unregister().await?;

        Ok::<_, Error>(())
    })?;
    println!("global shortcut probe passed: register, conflict, unregister, drop, re-register");

    Ok(())
}
