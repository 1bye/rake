//! Disposable native Console verification commands; compiled only with console-test.

use rake::prelude::*;

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "test.console", output = Output, capability = Headless)]
#[serde(deny_unknown_fields)]
struct Fixture {
    mode: String,
}

#[rake::data(schema)]
struct Output {
    message: String,
    nested: Vec<Detail>,
}

#[rake::data(schema)]
struct Detail {
    index: usize,
    text: String,
}

pub(super) fn register(commands: &Commands) {
    commands
        .register::<Fixture, _, _>(|fixture, _| async move {
            match fixture.mode.as_str() {
                "log" => {
                    rake::info!(items = 3, "Console fixture info");
                    rake::warn!(reason = "verification", "Console fixture warning");
                    rake::error!(reason = "intentional", "Console fixture error");
                    println!("Console fixture stdout");
                    eprintln!("Console fixture stderr");
                }
                "fail" => {
                    return Err(CommandError::Failed(
                        "Intentional Console fixture failure".into(),
                    ));
                }
                "slow" => rake::tasks::sleep(std::time::Duration::from_secs(3)).await,
                "crash" => {
                    std::thread::spawn(|| {
                        std::thread::sleep(std::time::Duration::from_millis(250));
                        let _ = std::panic::catch_unwind(|| {
                            panic!("Intentional Console fixture panic")
                        });
                        std::process::exit(73);
                    });
                }
                _ => return Err(CommandError::Failed("Use log, fail, slow or crash".into())),
            }
            Ok(Output {
                message: "café 你好 🙂".into(),
                nested: (0..3)
                    .map(|index| Detail {
                        index,
                        text: format!("item-{index}"),
                    })
                    .collect(),
            })
        })
        .expect("console fixture registration");
}
