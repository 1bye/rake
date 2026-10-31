//! Native no-initial-window host. A real tray app installs its platform handle here.
#[cfg(all(feature = "gpui", not(target_family = "wasm")))]
fn main() -> Result<(), rake::AppError> {
    rake::App::new("example.background")
        .background(|runtime, cx| {
            // Retain the UI-loop task until it has observed this managed job and quit.
            let task = cx.spawn(async move |_| {
                if let Ok(work) = runtime.tasks().spawn("background-work", async {
                    rake::tasks::sleep(std::time::Duration::from_millis(10)).await;

                    Ok(())
                }) {
                    let _ = work.wait().await;
                }

                drop(runtime.quit(rake::quit::QuitOptions::default()));
            });

            cx.set_global(Workflow { _task: task });
        })
        .run()
}

#[cfg(all(feature = "gpui", not(target_family = "wasm")))]
struct Workflow {
    _task: rake::gpui::Task<()>,
}

#[cfg(all(feature = "gpui", not(target_family = "wasm")))]
impl rake::gpui::Global for Workflow {}

#[cfg(any(not(feature = "gpui"), target_family = "wasm"))]
fn main() {
    eprintln!("the background example requires a native GPUI host");
}
