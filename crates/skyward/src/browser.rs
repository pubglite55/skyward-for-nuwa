//! Reading a page of the web on a thread of its own: the browser page asks
//! for one and is told when it has come, or why it did not.
//!
//! The address is only ever http or https, and only what a page says in its
//! markup is kept: the pictures it shows are taken too, as many as a recovery
//! can hold, and nothing a script would draw is run. What comes back is
//! either the page, as [`skyward_net::html`] reads it, with the PNG pictures
//! it shows, or the name of a file the address holds, which the browser takes
//! into the folder it saves to.

use anyhow::{Context, Result};
use skyward_net::download;
use skyward_net::html::{self, Block, Page};
use skyward_net::http;
use std::io::Read as _;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// How much of a page is read: what a recovery can show, and no more than
/// it can hold.
const MOST: u64 = 512 * 1024;

/// How many of a page's pictures are taken, and how much of each: enough for
/// the page to look like itself, and little enough to keep.
const MOST_PICTURES: usize = 3;
const MOST_PICTURE: u64 = 1024 * 1024;

/// What reading an address came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// A page, as its markup says it, with the pictures it shows that this
    /// recovery can read: each by the address it is at, and its PNG.
    Page { page: Page, pictures: Vec<(String, Vec<u8>)> },
    /// A file the address holds, which is taken into the folder the browser
    /// saves to, under this name.
    File { url: String, name: String },
}

/// An address being read, on its thread.
pub struct Fetcher {
    taken: Receiver<Result<Read, String>>,
}

impl Fetcher {
    /// Read `url` on a thread of its own, the names in it looked up in
    /// `servers`.
    pub fn start(url: String, servers: http::Servers) -> Result<Fetcher> {
        let (tx, taken) = mpsc::channel();
        std::thread::Builder::new()
            .name("browser".into())
            .spawn(move || {
                let _ = tx.send(fetch(&url, servers));
            })
            .context("start the thread reading the page")?;
        Ok(Fetcher { taken })
    }

    /// What reading came to since the last look, or why it came to nothing.
    pub fn taken(&self) -> Option<Result<Read, String>> {
        match self.taken.try_recv() {
            Ok(read) => Some(read),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }
}

fn fetch(url: &str, servers: http::Servers) -> Result<Read, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("地址需以 http:// 或 https:// 开头".into());
    }
    let agent = http::agent(servers);
    let mut answer = agent.get(url).call().map_err(|e| format!("{e}"))?;
    let status = answer.status();
    if !status.is_success() {
        return Err(format!("服务器回了 {status}"));
    }
    let kind = answer
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let disposition = answer.headers().get("content-disposition").and_then(|value| value.to_str().ok());
    if let Some(name) = file_name(&kind, url, disposition) {
        return Ok(Read::File { url: url.to_string(), name });
    }
    let mut body = Vec::new();
    answer
        .body_mut()
        .with_config()
        .limit(MOST)
        .reader()
        .read_to_end(&mut body)
        .map_err(|e| format!("读页面失败：{e}"))?;
    let markup = String::from_utf8_lossy(&body);
    let page = html::parse(&markup, url);
    let mut pictures = Vec::new();
    for block in &page.blocks {
        if pictures.len() >= MOST_PICTURES {
            break;
        }
        let Block::Image { to, .. } = block else { continue };
        if pictures.iter().any(|(at, _)| at == to) {
            continue;
        }
        if let Some(png) = picture(&agent, to) {
            pictures.push((to.clone(), png));
        }
    }
    Ok(Read::Page { page, pictures })
}

/// The PNG at `to`, where it is one and is no bigger than is kept: a picture
/// of another kind is left out, as it cannot be drawn here.
fn picture(agent: &ureq::Agent, to: &str) -> Option<Vec<u8>> {
    let mut answer = agent.get(to).call().ok()?;
    if !answer.status().is_success() {
        return None;
    }
    let kind = answer
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !kind.starts_with("image/png") {
        return None;
    }
    let mut body = Vec::new();
    answer.body_mut().with_config().limit(MOST_PICTURE).reader().read_to_end(&mut body).ok()?;
    (!body.is_empty()).then_some(body)
}

/// The name a file at `url` is taken under, where what came back is a file
/// and not a page: the one the server gives it, else the last of the address.
fn file_name(kind: &str, url: &str, disposition: Option<&str>) -> Option<String> {
    // What a page is said to be: its markup, or plain words. Anything else —
    // a package, an image, a video — is a file.
    let page = kind.starts_with("text/html") || kind.contains("xhtml") || kind.starts_with("text/plain");
    if page {
        return None;
    }
    if kind.is_empty() && ends_like_a_page(url) {
        return None;
    }
    if let Some(name) = disposition.and_then(named_by) {
        return Some(name);
    }
    Some(download::name_from(url))
}

/// Whether the address itself names a page, for a server that says nothing
/// of what it gives.
fn ends_like_a_page(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or_default();
    if last.is_empty() {
        return true;
    }
    match last.rsplit_once('.') {
        Some((_, ext)) => matches!(ext.to_ascii_lowercase().as_str(), "html" | "htm" | "txt" | "xhtml" | "php" | "asp" | "jsp"),
        None => true,
    }
}

/// The file name a `content-disposition` gives, as
/// `attachment; filename="a.zip"` gives it.
fn named_by(disposition: &str) -> Option<String> {
    let lower = disposition.to_ascii_lowercase();
    let at = lower.find("filename")?;
    let rest = &disposition[at + "filename".len()..];
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    let body = match quote {
        '"' | '\'' => &rest[1..],
        _ => rest,
    };
    let end = match quote {
        '"' | '\'' => body.find(quote)?,
        _ => body.find(';').unwrap_or(body.len()),
    };
    let name = body[..end].trim();
    // The name of a file to be written, not a path: only its last part is
    // taken.
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    (!name.is_empty() && name != "." && name != "..").then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_a_page_and_a_package_is_a_file() {
        assert_eq!(file_name("text/html; charset=utf-8", "https://x/a", None), None);
        assert_eq!(file_name("text/plain", "https://x/a", None), None);
        assert_eq!(file_name("application/zip", "https://x/a", None), Some("a".into()));
        assert_eq!(file_name("application/octet-stream", "https://x/dl/skyward.zip?x=1", None), Some("skyward.zip".into()));
    }

    #[test]
    fn a_server_that_says_nothing_is_judged_by_its_address() {
        assert_eq!(file_name("", "https://x/page.html", None), None);
        assert_eq!(file_name("", "https://x/page", None), None);
        assert_eq!(file_name("", "https://x/a.apk", None), Some("a.apk".into()));
    }

    #[test]
    fn the_name_a_server_gives_is_taken_over_the_address() {
        assert_eq!(file_name("application/zip", "https://x/a", Some("attachment; filename=\"b.zip\"")), Some("b.zip".into()));
        assert_eq!(file_name("application/zip", "https://x/a", Some("attachment; filename=c.zip; size=1")), Some("c.zip".into()));
        assert_eq!(file_name("application/zip", "https://x/a", Some("attachment; filename=\"../d.zip\"")), Some("d.zip".into()));
        assert_eq!(file_name("application/zip", "https://x/a", Some("attachment; filename=\"..\"")), Some("a".into()));
    }
}
