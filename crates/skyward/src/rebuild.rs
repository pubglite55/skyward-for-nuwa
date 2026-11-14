//! Building the data partition afresh where nothing is left of it.
//!
//! A phone builds the partition the first time its system starts: the
//! storage manager makes a key the secure world holds, wraps the partition's
//! own key with it, and keeps both in /metadata; the system then makes a
//! filesystem for that key to open. Nothing of that needs the system itself
//! — only the key service and the tools the recovery already carries — so a
//! partition a factory reset emptied is built here, and the system that
//! starts next finds it as it would have found its own.

use anyhow::{bail, Context, Result};
use skyward_block::{fstab, mount, BY_NAME};
use skyward_crypto::data;
use skyward_crypto::keymint::KeyMint;
use std::fs;
use std::path::Path;

/// Build the data partition afresh: make the key it is opened with, and give
/// it a filesystem. A partition that still holds anything, or one whose key
/// is already kept, is left as it is.
pub fn data(service: &KeyMint, force: bool) -> Result<()> {
    let entries = fstab::read(data::MOUNT)?;
    let entry = entries
        .iter()
        .find(|e| e.mount_point == data::MOUNT && e.extra("metadata_encryption").is_some())
        .context("the phone's table does not name a data partition it encrypts whole")?;
    let device = fstab::device(entry)?;
    let dir = data::key_dir(entry);
    // Asked for outright, what is there is thrown away first: the key and the
    // filesystem it opens are both being made anew.
    if !data::holds_nothing(Path::new(&device)) {
        if !force {
            bail!("{device} holds something already: name it a rebuild to throw it away, or empty it first");
        }
        skyward_block::erase::discard(Path::new(&device)).with_context(|| format!("empty {device}"))?;
    }
    if dir.exists() {
        if !force {
            bail!("{} already holds the key the partition is opened with", dir.display());
        }
        fs::remove_dir_all(&dir).with_context(|| format!("take away {}", dir.display()))?;
    }
    let metadata = Path::new("/metadata");
    if !mount::is_mount_point(metadata)? {
        mount::mount_ro(&Path::new(BY_NAME).join("metadata"), metadata).context("mount /metadata")?;
    }
    skyward_vab::metadata_writable(true).context("make /metadata writable")?;
    let built = (|| -> Result<()> {
        data::make_key(service, &dir)?;
        skyward_block::erase::make(Path::new(&device), &entry.filesystem, &entry.extras, false)
            .with_context(|| format!("make a {} filesystem on {device}", entry.filesystem))?;
        Ok(())
    })();
    // The metadata goes back to being read-only however the building went.
    let again = skyward_vab::metadata_writable(false);
    built?;
    again?;
    Ok(())
}
