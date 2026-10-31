//! Native archive output from a confined staged payload.
use crate::Error;
use std::{fs, path::Path};

pub(crate) fn zip(root: &Path, output: &Path) -> Result<(), Error> {
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;

    let mut archive = zip::ZipWriter::new(file);

    add(root, root, &mut archive)?;
    archive
        .finish()
        .map_err(|e| Error::Execution(e.to_string()))?;

    Ok(())
}

fn add(root: &Path, directory: &Path, archive: &mut zip::ZipWriter<fs::File>) -> Result<(), Error> {
    let mut entries: Vec<_> = fs::read_dir(directory)?.collect::<Result<_, _>>()?;

    entries.sort_by_key(fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();

        if entry.file_type()?.is_dir() {
            add(root, &path, archive)?;
        } else if entry.file_type()?.is_file() {
            let name = path
                .strip_prefix(root)
                .map_err(|e| Error::Execution(e.to_string()))?
                .to_string_lossy()
                .replace('\\', "/");

            archive
                .start_file(name, zip::write::SimpleFileOptions::default())
                .map_err(|e| Error::Execution(e.to_string()))?;
            std::io::copy(&mut fs::File::open(path)?, archive)?;
        } else {
            return Err(Error::Execution(
                "portable ZIP payload cannot contain links or special files".into(),
            ));
        }
    }

    Ok(())
}
