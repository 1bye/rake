//! Real AppKit run-loop probe; requires an available macOS graphical session.

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSDate, NSRunLoop, NSString};
    use std::sync::{Arc, Mutex};

    let mtm = MainThreadMarker::new().ok_or("probe must run on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    app.finishLaunching();
    let completed = Arc::new(Mutex::new(None));
    let result = completed.clone();
    let block = block2::RcBlock::new(move || {
        let outcome = probe().map_err(|error| error.to_string());
        *result.lock().expect("probe result mutex poisoned") = Some(outcome);
    });
    let run_loop = NSRunLoop::currentRunLoop();
    // SAFETY: The Send closure captures only synchronized result storage and is
    // scheduled on the current main run loop. Native resources are created there.
    unsafe {
        run_loop.performBlock(&block);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);

    while completed
        .lock()
        .expect("probe result mutex poisoned")
        .is_none()
    {
        if std::time::Instant::now() >= deadline {
            return Err("AppKit run-loop probe timed out".into());
        }

        run_loop.runMode_beforeDate(
            &NSString::from_str("kCFRunLoopDefaultMode"),
            &NSDate::dateWithTimeIntervalSinceNow(0.1),
        );
    }

    completed
        .lock()
        .expect("probe result mutex poisoned")
        .take()
        .ok_or("probe produced no result")??;
    println!(
        "native tray probe passed: two items, geometry, update, hide/show, remove/recreate, weak owner shutdown"
    );

    Ok(())
}

#[cfg(target_os = "macos")]
fn probe() -> Result<(), Box<dyn std::error::Error>> {
    use futures::executor::block_on;
    use rake_tray::{Error, Host, Image, ImageRepresentation, Tray};

    let image = Image::new(ImageRepresentation::rgba(18, 18, vec![255; 18 * 18 * 4])?);
    let owner = Host::new("dev.rake.tray-probe")?;
    let collection = owner.collection();
    let first = block_on(
        collection.create(
            Tray::new("first")
                .icon(image.clone())
                .tooltip("First probe item"),
        ),
    )?;
    let second = block_on(
        collection.create(
            Tray::new("second")
                .icon(image.clone())
                .tooltip("Second probe item"),
        ),
    )?;

    if collection.items()?.len() != 2 {
        return Err("two independent registrations were not retained".into());
    }
    if first.anchor()?.and_then(|a| a.rect).is_none() {
        return Err("AppKit did not supply native icon geometry".into());
    }

    block_on(first.set_tooltip("Updated first item"))?;
    block_on(first.hide())?;
    block_on(first.show())?;
    block_on(first.remove())?;
    let replacement = block_on(collection.create(Tray::new("first").icon(image)))?;
    block_on(first.remove())?;
    replacement.state()?;
    second.state()?;
    block_on(owner.shutdown())?;

    if !matches!(replacement.state(), Err(Error::Stopped)) {
        return Err("weak reference survived owner shutdown".into());
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("this probe requires macOS; use the native runner for the selected platform");
    std::process::exit(1);
}
