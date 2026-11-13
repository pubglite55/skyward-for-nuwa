//! The network the WLAN password is kept for, so that the start after this
//! one joins it without asking: kept on /metadata beside the settings,
//! readable by the recovery alone, and dropped as soon as the network
//! refuses the password.
//!
//! Whether the WLAN comes up as the recovery starts, and whether the
//! password is kept at all, are the pages' switches, kept with the rest of
//! the settings; this is only the network and the password itself.

use crate::settings::with_metadata;
use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::Write;

const FOLDER: &str = "/metadata/skyward";
const FILE: &str = "/metadata/skyward/wlan";
/// What a keep goes into first, to be moved over the file whole.
const WRITING: &str = "/metadata/skyward/wlan.new";

/// A network kept, with the password it was joined with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub ssid: Vec<u8>,
    pub password: String,
}

/// The network kept, if the phone keeps one.
pub fn read() -> Option<Saved> {
    let text = fs::read_to_string(FILE).ok()?;
    parse(&text)
}

/// Keep `ssid` and the password it was joined with.
pub fn save(ssid: &[u8], password: &str) -> Result<()> {
    let text = render(ssid, password);
    with_metadata(|| {
        skyward_vab::metadata_writable(true)?;
        let wrote = write_file(&text);
        let back = skyward_vab::metadata_writable(false);
        wrote?;
        back
    })
}

/// Drop the network kept, if there is one: what refused the password is not
/// kept, and neither is what the pages are no longer set to keep.
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

fn write_file(text: &str) -> Result<()> {
    fs::create_dir_all(FOLDER).with_context(|| format!("make {FOLDER}"))?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Only the recovery reads it, and only when it is joining.
    #[cfg(target_os = "android")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(WRITING).with_context(|| format!("make {WRITING}"))?;
    file.write_all(text.as_bytes()).with_context(|| format!("write {WRITING}"))?;
    file.sync_all().with_context(|| format!("write {WRITING}"))?;
    fs::rename(WRITING, FILE).with_context(|| format!("move {WRITING} over {FILE}"))?;
    File::open(FOLDER).and_then(|folder| folder.sync_all()).with_context(|| format!("write {FOLDER}"))
}

/// What is kept, as the file holds it: the name as bytes in hexadecimal, so
/// that a name of any alphabet goes round the file whole, and the password
/// as it was typed.
fn render(ssid: &[u8], password: &str) -> String {
    format!("ssid={}\npassword={password}\n", hex(ssid))
}

fn parse(text: &str) -> Option<Saved> {
    let (mut ssid, mut password) = (None, None);
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "ssid" => ssid = unhex(value.trim()),
            "password" => password = Some(value.to_string()),
            _ => {}
        }
    }
    let (ssid, password) = (ssid?, password?);
    (!ssid.is_empty() && !password.is_empty()).then_some(Saved { ssid, password })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_goes_round_the_file_as_bytes() {
        assert_eq!(unhex(&hex("Skyward客厅".as_bytes())).as_deref(), Some("Skyward客厅".as_bytes()));
    }

    #[test]
    fn a_password_keeps_what_it_holds() {
        let saved = "pa ss=word";
        assert_eq!(parse(&render(b"Skyward", saved)), Some(Saved { ssid: b"Skyward".to_vec(), password: saved.into() }));
    }

    #[test]
    fn half_a_byte_is_no_name() {
        assert_eq!(unhex("abc"), None);
        assert_eq!(parse("ssid=abc\npassword=x\n"), None);
    }

    #[test]
    fn a_name_or_password_left_empty_is_not_kept() {
        assert_eq!(parse("ssid=\npassword=x\n"), None);
        assert_eq!(parse("ssid=53\npassword=\n"), None);
        assert_eq!(parse("ssid=53\n"), None);
    }
}
