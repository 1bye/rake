//! Bundled build/watch supervisor, sharing the repository's standard-library Web tooling.

use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;

pub(crate) fn run(arguments: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!("rake-tools-{}", uuid::Uuid::new_v4()));

    std::fs::DirBuilder::new().mode(0o700).create(&directory)?;

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        for (name, source) in [
            ("dev.py", include_str!("../../../scripts/dev.py")),
            (
                "serve-web.py",
                include_str!("../../../scripts/serve-web.py"),
            ),
            (
                "dev_bridge.py",
                include_str!("../../../scripts/dev_bridge.py"),
            ),
        ] {
            std::fs::write(directory.join(name), source)?;
        }

        let error = std::process::Command::new("python3")
            .arg(directory.join("dev.py"))
            .args(arguments)
            .env("RAKE_EMBEDDED_HELPERS", &directory)
            .exec();

        Err(error.into())
    })();

    let _ = std::fs::remove_dir_all(directory);

    result
}
