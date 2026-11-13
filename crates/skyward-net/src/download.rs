//! Downloading a package whole: from where an earlier download of it
//! stopped, over connections that may drop, and checked against the digest
//! its server gave before it takes its own name.

use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use sha2::Sha256;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// How much is read and written at a time.
const CHUNK: usize = 1 << 20;

/// The digest a package's server gives of it, in hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sum {
    Md5(String),
    Sha256(String),
    /// An address that says no digest, as a link typed by hand does not:
    /// what comes is taken as it is.
    None,
}

/// Where a package is downloaded to until it is whole and checked.
pub fn partial(path: &Path) -> PathBuf {
    let mut name = path.file_name().expect("a package has a name").to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

/// The name a link's file goes by: what its address ends with, its percent
/// signs read, and a name of its own where the address ends with none.
pub fn name_from(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    // What follows the host: an address that ends at the host, or at a
    // slash that closes a folder, names no file.
    let after_host = match path.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("", |at| &rest[at + 1..]),
        None => path,
    };
    if after_host.is_empty() || after_host.ends_with('/') {
        return "download.bin".to_string();
    }
    let last = after_host.rsplit('/').next().unwrap_or("");
    let name: String = percent_decode(last).chars().filter(|c| *c != '/' && *c != '\\' && !c.is_control()).collect();
    match name.as_str() {
        "" | "." | ".." => "download.bin".to_string(),
        _ => name,
    }
}

/// A name in `dir` no file has yet: the one asked for where it is free, and
/// the same with a number after it otherwise.
pub fn free_name(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    for n in 2..1000 {
        let candidate = format!("{stem} ({n}){extension}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{stem} ({}){extension}", std::process::id())
}

/// What a percent-encoded part of an address says. A plus stays a plus:
/// it stands for a space in a query, and for itself in a file's name.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// How far a download has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// What an earlier download left is read for its digest, before the
    /// rest is added to it.
    Checking { done: u64, total: u64 },
    Downloading { done: u64, total: u64 },
}

/// Why a download did not end with the package whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The folder has too little room for what is left of it.
    NoRoom { needed: u64, free: u64 },
    /// What was downloaded was not the package its server described, and
    /// is gone.
    Damaged,
    /// The server now offers another system than the one downloaded.
    Changed,
    /// The connection kept failing, or anything else: why.
    Other(String),
}

/// The server turned the download down: what a signed address that has
/// run out gets, or one it no longer serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused(pub u16);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the server answered {}", self.0)
    }
}

impl std::error::Error for Refused {}

/// The whole package does not have the digest its server gave; what was
/// downloaded of it is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damaged;

impl std::fmt::Display for Damaged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the package downloaded does not have the digest its server gave")
    }
}

impl std::error::Error for Damaged {}

/// The folder has too little room for what is left to download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoRoom {
    pub needed: u64,
    pub free: u64,
}

impl std::fmt::Display for NoRoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} bytes more are needed, and {} are free", self.needed, self.free)
    }
}

impl std::error::Error for NoRoom {}

/// The bytes free for anyone to write in the filesystem `dir` is on.
pub fn free(dir: &Path) -> Result<u64> {
    let c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).context("a path with a NUL")?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| format!("read the free space of {}", dir.display()));
    }
    Ok(s.f_bavail as u64 * s.f_frsize as u64)
}

/// The digest, by `D`, of what `file` holds, read from its start.
fn hash_of<D: Digest>(file: &mut File, total: u64, stop: &AtomicBool, progress: &mut dyn FnMut(Progress)) -> Result<D> {
    let mut hash = D::new();
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0;
    loop {
        if stop.load(Ordering::Relaxed) {
            bail!(crate::Stopped);
        }
        let n = file.read(&mut buf).context("read what was downloaded before")?;
        if n == 0 {
            return Ok(hash);
        }
        hash.update(&buf[..n]);
        done += n as u64;
        progress(Progress::Checking { done, total });
    }
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Download `url` to `path`, carrying on from what an earlier download
/// left, and give it its name once it has the digest `sum`. A part as long
/// as the package is only checked; a server that will not send from the
/// middle sends it all again.
pub fn fetch(agent: &ureq::Agent, url: &str, path: &Path, sum: &Sum, stop: &AtomicBool, progress: &mut dyn FnMut(Progress)) -> Result<()> {
    match sum {
        Sum::Md5(hex) => fetch_as::<Md5>(agent, url, path, Some(hex), stop, progress),
        Sum::Sha256(hex) => fetch_as::<Sha256>(agent, url, path, Some(hex), stop, progress),
        Sum::None => fetch_as::<Sha256>(agent, url, path, None, stop, progress),
    }
}

/// [`fetch`], with the digest by `D` in hex where one is checked.
fn fetch_as<D: Digest>(agent: &ureq::Agent, url: &str, path: &Path, sum: Option<&str>, stop: &AtomicBool, progress: &mut dyn FnMut(Progress)) -> Result<()> {
    let part = partial(path);
    let dir = path.parent().context("a package's path is in a folder")?;
    fs::create_dir_all(dir).with_context(|| format!("make {}", dir.display()))?;
    let head = agent.head(url).call().context("ask the server about the package")?;
    match head.status().as_u16() {
        200 => {}
        status => bail!(Refused(status)),
    }
    let total = content_length(head.headers()).context("the server does not say how long the package is")?;
    let mut file = OpenOptions::new().create(true).read(true).append(true).open(&part).with_context(|| format!("open {}", part.display()))?;
    let mut have = file.metadata().with_context(|| format!("read {}", part.display()))?.len();
    if have > total {
        file.set_len(0).with_context(|| format!("empty {}", part.display()))?;
        have = 0;
    }
    let room = free(dir)?;
    if total - have > room {
        bail!(NoRoom { needed: total - have, free: room });
    }
    // What an earlier download left is read for its digest only where a
    // digest is checked.
    let mut hash = match sum {
        Some(_) => Some(hash_of::<D>(&mut file, have, stop, progress)?),
        None => None,
    };
    if have < total {
        let mut asked = agent.get(url);
        if have > 0 {
            asked = asked.header("Range", format!("bytes={have}-"));
        }
        let mut response = asked.call().context("start the download")?;
        match response.status().as_u16() {
            206 if have > 0 => {}
            200 => {
                if have > 0 {
                    file.set_len(0).with_context(|| format!("empty {}", part.display()))?;
                    have = 0;
                    if hash.is_some() {
                        hash = Some(D::new());
                    }
                }
            }
            status => bail!(Refused(status)),
        }
        let mut body = response.body_mut().with_config().limit(u64::MAX).reader();
        let mut buf = vec![0u8; CHUNK];
        let mut done = have;
        progress(Progress::Downloading { done, total });
        while done < total {
            if stop.load(Ordering::Relaxed) {
                bail!(crate::Stopped);
            }
            let n = body.read(&mut buf).context("download the package")?;
            if n == 0 {
                bail!("the server stopped sending at {done} of {total} bytes");
            }
            let n = n.min((total - done) as usize);
            file.write_all(&buf[..n]).with_context(|| format!("write {}", part.display()))?;
            if let Some(hash) = hash.as_mut() {
                hash.update(&buf[..n]);
            }
            done += n as u64;
            progress(Progress::Downloading { done, total });
        }
        file.sync_all().with_context(|| format!("write {} through", part.display()))?;
    }
    if let Some(sum) = sum {
        let got = hex(&hash.expect("a digest is kept where one is checked").finalize());
        if got != sum.to_lowercase() {
            drop(file);
            fs::remove_file(&part).with_context(|| format!("delete {}", part.display()))?;
            bail!(Damaged);
        }
    }
    fs::rename(&part, path).with_context(|| format!("name {} {}", part.display(), path.display()))
}

/// The length a response says its body has.
pub(crate) fn content_length(headers: &ureq::http::HeaderMap) -> Option<u64> {
    headers.get(ureq::http::header::CONTENT_LENGTH)?.to_str().ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_is_named_after_its_package() {
        assert_eq!(partial(Path::new("/sdcard/downloaded_rom/a.zip")), Path::new("/sdcard/downloaded_rom/a.zip.part"));
    }

    #[test]
    fn a_link_names_its_file_by_what_it_ends_with() {
        assert_eq!(name_from("https://a.example/rom/lineage.zip"), "lineage.zip");
        assert_eq!(name_from("https://a.example/rom/a%20b.zip?token=1"), "a b.zip", "the query stays out of the name");
        assert_eq!(name_from("https://a.example/rom/a+b.zip"), "a+b.zip", "a plus is a plus in a name");
        assert_eq!(name_from("https://a.example/rom/"), "download.bin", "an address ending with a slash names nothing");
        assert_eq!(name_from("https://a.example"), "download.bin");
        assert_eq!(name_from("https://a.example/rom/.."), "download.bin");
    }

    #[test]
    fn a_name_taken_takes_a_number() {
        let dir = std::env::temp_dir().join(format!("skyward-net-name-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_name(&dir, "a.zip"), "a.zip");
        fs::write(dir.join("a.zip"), b"x").unwrap();
        assert_eq!(free_name(&dir, "a.zip"), "a (2).zip");
        fs::write(dir.join("a (2).zip"), b"x").unwrap();
        assert_eq!(free_name(&dir, "a.zip"), "a (3).zip");
        assert_eq!(free_name(&dir, "README"), "README", "a name with no ending of its own still goes in");
        fs::write(dir.join("README"), b"x").unwrap();
        assert_eq!(free_name(&dir, "README"), "README (2)");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_is_there_is_read_for_its_digest() {
        let dir = std::env::temp_dir().join(format!("skyward-net-hash-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.part");
        fs::write(&path, vec![7u8; CHUNK + 5]).unwrap();
        let mut seen = Vec::new();
        let hash = hash_of::<Md5>(&mut File::open(&path).unwrap(), (CHUNK + 5) as u64, &AtomicBool::new(false), &mut |p| seen.push(p)).unwrap();
        assert_eq!(hex(&hash.finalize()), hex(&Md5::digest(vec![7u8; CHUNK + 5])));
        assert_eq!(seen.last(), Some(&Progress::Checking { done: (CHUNK + 5) as u64, total: (CHUNK + 5) as u64 }));
        let hash = hash_of::<Sha256>(&mut File::open(&path).unwrap(), (CHUNK + 5) as u64, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(hex(&hash.finalize()), hex(&Sha256::digest(vec![7u8; CHUNK + 5])));
        let stopped = hash_of::<Md5>(&mut File::open(&path).unwrap(), 1, &AtomicBool::new(true), &mut |_| {});
        assert!(stopped.unwrap_err().is::<crate::Stopped>());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn free_space_is_read_from_the_filesystem() {
        assert!(free(&std::env::temp_dir()).unwrap() > 0);
        assert!(free(Path::new("/no/such/folder")).is_err());
    }
}
