//! Replace the CLI process with its bundled client, keeping arguments out of code.

use std::os::unix::process::CommandExt;

pub(super) fn run(mut args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    args.remove(0);
    let script = concat!(
        include_str!("../../../scripts/rake-agent.mjs"),
        "\nawait cli(process.argv.slice(1)).catch(error => { console.error(error.message); process.exitCode = 1; });"
    );
    // exec preserves signal/exit ownership: interruption cannot orphan a Node child.
    let error = std::process::Command::new("node")
        .args(["--input-type=module", "--eval", script, "--"])
        .args(args)
        .exec();

    Err(format!("agent client requires Node 22+: {error}").into())
}
