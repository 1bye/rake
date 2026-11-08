//! Run the bundled Node client with owned signal and process lifetimes.

#[cfg(unix)]
use std::os::unix::process::CommandExt;

pub(super) fn run(mut args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    args.remove(0);

    #[cfg(unix)]
    let error = {
        let script = concat!(
            include_str!("../scripts/rake-agent.mjs"),
            "\nawait cliMain(process.argv.slice(1));"
        );

        // exec preserves signal/exit ownership without leaving a Node child.
        std::process::Command::new("node")
            .args(["--input-type=module", "--eval", script, "--"])
            .args(args)
            .exec()
    };

    #[cfg(windows)]
    let error = {
        // Keep the embedded script out of Windows' bounded command line. A file
        // entry point adds its filename to argv, hence slice(2) rather than (1).
        let directory = tempfile::Builder::new().prefix("rake-agent-").tempdir()?;
        let script = directory.path().join("agent.mjs");

        std::fs::write(
            &script,
            concat!(
                include_str!("../scripts/rake-agent.mjs"),
                "\nawait cliMain(process.argv.slice(2));"
            ),
        )?;
        let mut command = std::process::Command::new("node");

        command.arg(&script).args(args);
        let result = crate::project_build::run_client(&mut command);

        drop(directory);

        match result {
            Ok(code) => std::process::exit(code),
            Err(error) => std::io::Error::other(error.to_string()),
        }
    };

    Err(format!("agent client requires Node 22+: {error}").into())
}
