//! A page on the network the phone joined, where the address of a file is
//! typed on another device and handed to this one: a link too long for the
//! recovery's own keyboard comes in over the network instead.

use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// The port the page is served on.
pub const PORT: u16 = 8081;

/// The page, and the link another device passed through it.
pub struct Portal {
    url: String,
    taken: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
}

impl Portal {
    /// Serve the page on every address this phone has, reachable at `host`.
    pub fn start(host: &str) -> Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", PORT)).with_context(|| format!("listen on port {PORT}"))?;
        // Nothing waits in accept: a thread parked there would keep the port
        // bound after the page is left, and the next visit could not serve it.
        listener.set_nonblocking(true)?;
        let taken: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (slot, flag) = (taken.clone(), stop.clone());
        thread::Builder::new()
            .name("portal".into())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if let Err(e) = serve(stream, &slot) {
                                eprintln!("the page for a passed link: {e:#}");
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(80)),
                        Err(_) => thread::sleep(Duration::from_millis(80)),
                    }
                }
            })
            .context("start the thread serving the page")?;
        Ok(Self { url: format!("http://{host}:{PORT}/"), taken, stop })
    }

    /// The address another device opens.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The link passed since the last look, if any.
    pub fn take(&self) -> Option<String> {
        self.taken.lock().ok().and_then(|mut slot| slot.take())
    }
}

impl Drop for Portal {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn serve(mut stream: TcpStream, slot: &Arc<Mutex<Option<String>>>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 8192 {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buf);
    let first = text.lines().next().unwrap_or("");
    let target = first.split_whitespace().nth(1).unwrap_or("/");
    let delivered = "<!doctype html><meta charset=utf-8><body style=\"font:17px system-ui;background:#111;color:#eee;padding:28px\"><h2>已送达</h2><p>回到恢复界面，链接会自己填进地址栏。</p><p><a style=\"color:#6cf\" href=\"/\">再送一个</a></p></body>";
    let (code, reason, body) = match target.strip_prefix("/set?") {
        Some(query) => match field(query, "url") {
            Some(link) if !link.trim().is_empty() => {
                if let Ok(mut slot) = slot.lock() {
                    *slot = Some(link.trim().to_string());
                }
                (303u16, "See Other", delivered.to_string())
            }
            _ => (200, "OK", form("链接是空的")),
        },
        None => (200, "OK", form("")),
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    let _ = stream.flush();
    Ok(())
}

/// The page another device is handed, where the address is typed.
fn form(note: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\">\
<title>Skyward 送一个链接</title>\
<body style=\"font:17px system-ui;background:#111;color:#eee;padding:24px\">\
<h2 style=\"margin:0 0 4px\">送给恢复的链接</h2>\
<p style=\"color:#888;margin:0 0 20px\">在这台手机上粘贴或输入一个网址，发送后恢复那边的地址栏就会填上。</p>\
<form method=get action=/set>\
<input name=url autocomplete=off autocapitalize=off spellcheck=false placeholder=\"https://\" \
style=\"width:100%;box-sizing:border-box;font-size:17px;padding:14px;border-radius:12px;border:1px solid #333;background:#1b1b22;color:#eee\">\
<button style=\"margin-top:18px;font-size:17px;padding:14px 26px;border:0;border-radius:999px;background:#5b8cff;color:#fff\">发送</button>\
</form>\
<p style=\"color:#f66;margin-top:16px\">{note}</p></body>"
    )
}

/// The value of a field in a query string, percent-decoded.
fn field(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then(|| decode(value))
    })
}

/// `%XX` and `+` as a browser writes them.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                Ok(byte) => {
                    out.push(byte);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_arrives_as_the_browser_wrote_it() {
        assert_eq!(field("url=https%3A%2F%2Fexample.com%2Fa.zip", "url").as_deref(), Some("https://example.com/a.zip"));
        assert_eq!(field("x=1&url=a+b", "url").as_deref(), Some("a b"));
        assert_eq!(field("url=%E4%B8%AD", "url").as_deref(), Some("中"));
    }

    #[test]
    fn a_field_that_is_not_there_is_none() {
        assert_eq!(field("x=1", "url"), None);
        assert_eq!(field("", "url"), None);
    }

    #[test]
    fn half_a_percent_sign_is_left_as_it_is() {
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
    }
}
