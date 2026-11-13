//! The credential a user chose to have kept, so that the start after this
//! one opens their own files without asking for it again: kept on /metadata
//! beside the settings, readable by the recovery alone, and dropped as soon
//! as it stops opening them.

use crate::settings::with_metadata;
use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::Write;

const FOLDER: &str = "/metadata/skyward";
const FILE: &str = "/metadata/skyward/credential";
/// What a keep goes into first, to be moved over the file whole.
const WRITING: &str = "/metadata/skyward/credential.new";

/// The credential kept, if the phone keeps one.
pub fn read() -> Result<Option<Vec<u8>>> {
    with_metadata(|| match fs::read(FILE) {
        Ok(credential) => Ok(Some(credential)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {FILE}")),
    })
}

/// Keep `credential` for the start after this one.
pub fn keep(credential: &[u8]) -> Result<()> {
    with_metadata(|| {
        skyward_vab::metadata_writable(true)?;
        let wrote = write_file(credential);
        let back = skyward_vab::metadata_writable(false);
        wrote?;
        back
    })
}

/// Drop the credential kept, if there is one: what no longer opens the files
/// is not kept, and what the pages are no longer set to keep is not either.
pub fn forget() -> Result<()> {
    with_metadata(|| {
        skyward_vab::metadata_writable(true)?;
        let gone = match fs::remove_file(FILE) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("delete {FILE}")),
        };
        let back = skyward_vab::metadata_writable(false);
        gone?;
        back
    })
}

fn write_file(credential: &[u8]) -> Result<()> {
    fs::create_dir_all(FOLDER).with_context(|| format!("make {FOLDER}"))?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Only the recovery reads it, and only when it is opening the files.
    #[cfg(target_os = "android")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(WRITING).with_context(|| format!("make {WRITING}"))?;
    file.write_all(credential).with_context(|| format!("write {WRITING}"))?;
    file.sync_all().with_context(|| format!("write {WRITING}"))?;
    fs::rename(WRITING, FILE).with_context(|| format!("move {WRITING} over {FILE}"))?;
    File::open(FOLDER).and_then(|folder| folder.sync_all()).with_context(|| format!("write {FOLDER}"))
}
