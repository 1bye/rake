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
    use rake_tray::{
        CheckState, Error, EventKind, Host, Image, ImageRepresentation, ItemKind, Menu, MenuItem,
        Tray,
    };

    let image = Image::new(ImageRepresentation::rgba(18, 18, vec![255; 18 * 18 * 4])?);
    let owner = Host::new("dev.rake.tray-probe")?;
    let collection = owner.collection();
    let menu = Menu::new()
        .item(MenuItem::new("open", "Open & inspect_"))
        .item(MenuItem::new("mixed", "Mixed").kind(ItemKind::Check(CheckState::Mixed)))
        .item(MenuItem::new("nested", "Nested").kind(ItemKind::Submenu(
            Menu::new().item(MenuItem::quit("quit", "Quit")),
        )));
    let first = block_on(
        collection.create(
            Tray::new("first")
                .icon(image.clone())
                .tooltip("First probe item")
                .menu(menu),
        ),
    )?;
    let second = block_on(
        collection.create(
            Tray::new("second")
                .icon(image.clone())
                .tooltip("Second probe item"),
        ),
    )?;

    let text = block_on(collection.create(Tray::new("text-only").macos(
        rake_tray::macos::Appearance {
            title: "42%".into(),
            accessibility_label: "Probe progress".into(),
            length: rake_tray::macos::ItemLength::Points(60.0),
            ..Default::default()
        },
    )))?;
    if text.macos_metrics()?.length != 60.0 {
        return Err("explicit AppKit length was not applied".into());
    }
    block_on(text.remove())?;
    if collection.items()?.len() != 2 {
        return Err("two independent registrations were not retained".into());
    }
    if first.anchor()?.and_then(|a| a.rect).is_none() {
        return Err("AppKit did not supply native icon geometry".into());
    }

    first.probe_menu_selection(0)?;
    let selected = block_on(collection.next_event())?;
    if selected.kind != EventKind::MenuSelected("open".into()) {
        return Err("native menu target/action did not reach the event consumer".into());
    }
    block_on(first.set_tooltip("Updated first item"))?;
    first.probe_menu_selection(0)?;
    if block_on(collection.next_event())?.menu_revision != Some(1) {
        return Err("native menu projection revision was not updated".into());
    }
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
