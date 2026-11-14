//! Giving the data partition a new filesystem, and throwing away the key it
//! was opened with.
//!
//! A phone builds the partition the first time its system starts: the
//! storage manager has the secure world make a key, wraps the partition's
//! own key with it, keeps both in /metadata, and makes the filesystem the
//! key opens. The key is the secure world's to make, and the key service of
//! this phone takes the phone down when the recovery asks it for one — twice
//! seen, both times ending in a phone that had to be started again by hand.
//! So the recovery does the half it can do safely: it empties the partition
//! and throws the old key away, and the system that starts next makes the
//! key and the filesystem, as it does on its own first start.

use anyhow::{bail, Context, Result};
use skyward_block::{fstab, mount, BY_NAME};
use skyward_crypto::data;
use std::fs;
use std::path::Path;

/// Give the data partition a new filesystem and throw away the key it was
/// opened with. A partition that still holds anything is left as it is,
/// unless `force` says to throw that away too.
pub fn data(force: bool) -> Result<()> {
    let entries = fstab::read(data::MOUNT)?;
    let entry = entries
        .iter()
        .find(|e| e.mount_point == data::MOUNT && e.extra("metadata_encryption").is_some())
        .context("the phone's table does not name a data partition it encrypts whole")?;
    let device = fstab::device(entry)?;
    let dir = data::key_dir(entry);
    // The filesystem is made over whatever is there; the whole partition is
    // not discarded first. A discard of every block of it stalls the storage
    // — the phone was seen to go down under one — and the filesystem being
    // made afresh loses what was there just the same.
    if !data::holds_nothing(Path::new(&device)) && !force {
        bail!("{device} holds something already: say it is to be given a new filesystem to throw it away");
    }
    let metadata = Path::new("/metadata");
    if !mount::is_mount_point(metadata)? {
        mount::mount_ro(&Path::new(BY_NAME).join("metadata"), metadata).context("mount /metadata")?;
    }
    skyward_vab::metadata_writable(true).context("make /metadata writable")?;
    let built = (|| -> Result<()> {
        // A key kept for a partition that is no longer there would keep the
        // system from making the key that opens the new one.
        if dir.exists() {
            fs::remove_dir_all(&dir).with_context(|| format!("take away {}", dir.display()))?;
        }
        skyward_block::erase::make(Path::new(&device), &entry.filesystem, &entry.extras, false)
            .with_context(|| format!("make a {} filesystem on {device}", entry.filesystem))?;
        Ok(())
    })();
    // The metadata goes back to being read-only however it went.
    let again = skyward_vab::metadata_writable(false);
    built?;
    again?;
    Ok(())
}
