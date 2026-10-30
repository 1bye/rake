//! Disposable real allocator workload; present only in the memory-test example.

use rake::prelude::*;
use std::sync::{Mutex, OnceLock};

static BLOCKS: OnceLock<Mutex<Vec<Vec<u8>>>> = OnceLock::new();

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "test.memory", output = Output, capability = Headless)]
#[serde(deny_unknown_fields)]
struct Workload {
    action: String,
}
#[rake::data(schema)]
struct Output {
    blocks: usize,
    bytes: usize,
}

#[inline(never)]
fn allocate_probe() -> Vec<u8> {
    std::hint::black_box(vec![42; 1024 * 1024])
}

pub(super) fn register(commands: &Commands) {
    commands
        .register::<Workload, _, _>(|command, _| async move {
            let mut blocks = BLOCKS
                .get_or_init(Mutex::default)
                .lock()
                .expect("fixture blocks lock poisoned");
            match command.action.as_str() {
                "allocate" => {
                    if blocks.len() >= 16 {
                        return Err(CommandError::Failed(
                            "Release the fixture's retained blocks first".into(),
                        ));
                    }
                    for _ in 0..8 {
                        blocks.push(allocate_probe());
                    }
                }
                "release" => blocks.clear(),
                "churn" => {
                    for _ in 0..8 {
                        drop(allocate_probe());
                    }
                }
                _ => {
                    return Err(CommandError::Failed(
                        "Use allocate, release or churn".into(),
                    ));
                }
            }
            Ok(Output {
                blocks: blocks.len(),
                bytes: blocks.iter().map(Vec::len).sum(),
            })
        })
        .expect("memory fixture registration");
}
