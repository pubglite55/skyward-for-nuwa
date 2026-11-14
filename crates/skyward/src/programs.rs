//! The programs the key service is made of, as the phone's own init scripts
//! describe them: read at start, so that a system built differently from the
//! one this fork's configuration was written on still opens its data.

use skyward_config::{programs, Service};
use std::fs;

/// The folders whose init scripts describe the phone's own programs. The
/// recovery's own ramdisk has one too, and its programs are its own.
const FOLDERS: [&str; 4] = ["/vendor/etc/init", "/odm/etc/init", "/system_ext/etc/init", "/system/etc/init"];

/// The key service's programs, in the order they run, as the phone's own
/// system describes them; none where it describes none of them.
pub fn key_chain() -> Vec<Service> {
    let mut found = Vec::new();
    for folder in FOLDERS {
        let Ok(entries) = fs::read_dir(folder) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            // Only the scripts: a folder may hold a file of another kind.
            if path.extension().and_then(|extension| extension.to_str()) != Some("rc") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else { continue };
            found.extend(programs::parse(&text));
        }
    }
    programs::in_order(found)
}
