//! A server on the network the phone is on, for the files in a folder: a
//! browser elsewhere lists them, takes them, and puts others there.
//!
//! It answers on every address the phone has, so that a computer that
//! reached the phone through a router and one joined to the phone's own
//! hotspot both find it.

use anyhow::{bail, Context, Result};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// The port the files are served on.
pub const PORT: u16 = 8080;

/// The most a request's head may take.
const HEAD_MAX: usize = 16 << 10;

/// How long a client may take over what it sends or reads before it is let
/// go, so that one that goes away does not hold a thread for good.
const PATIENCE: Duration = Duration::from_secs(120);

/// How much is read or written at a time.
const CHUNK: usize = 64 << 10;

/// What the server is doing, as the page shows it.
pub struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
    served: Arc<AtomicU64>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    /// Serve the files under `root` on [`PORT`], until the server is let go.
    pub fn start(root: PathBuf) -> Result<Server> {
        Server::start_at(root, PORT)
    }

    /// Serve on `port`, which is the one asked for where it is free.
    fn start_at(root: PathBuf, port: u16) -> Result<Server> {
        // The folder is named once with every link along the way followed,
        // so that what a browser asks for is checked against what is served
        // rather than against a name that leads elsewhere.
        let root = root.canonicalize().with_context(|| format!("serve {}", root.display()))?;
        let listener = TcpListener::bind(("0.0.0.0", port)).with_context(|| format!("listen on port {port}"))?;
        let port = listener.local_addr()?.port();
        // Nothing waits in accept: a thread parked there would keep the port
        // bound after the page is left, and coming back could not serve it.
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let served = Arc::new(AtomicU64::new(0));
        let (flag, count) = (stop.clone(), served.clone());
        let thread = thread::Builder::new()
            .name("files".into())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let root = root.clone();
                            let count = count.clone();
                            // Each request is answered on a thread of its
                            // own, so that a long transfer holds up nothing.
                            let _ = thread::Builder::new().name("files-request".into()).spawn(move || {
                                let _ = answer(stream, &root);
                                count.fetch_add(1, Ordering::Relaxed);
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(80)),
                        Err(_) => thread::sleep(Duration::from_millis(80)),
                    }
                }
            })
            .context("start the thread serving the files")?;
        Ok(Server { port, stop, served, thread: Some(thread) })
    }

    /// The port the files are served on: the one asked for, where it was
    /// free.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// How many requests have been answered.
    pub fn served(&self) -> u64 {
        self.served.load(Ordering::Relaxed)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One request's head: what it asks, and what it says of what it carries.
struct Head {
    method: String,
    path: String,
    /// How many bytes a body carries, where the request says.
    length: Option<u64>,
    /// What part of a file a download asks for.
    range: Option<String>,
}

/// Answer one request on `stream`, for the files under `root`.
fn answer(mut stream: TcpStream, root: &Path) -> Result<()> {
    stream.set_read_timeout(Some(PATIENCE))?;
    stream.set_write_timeout(Some(PATIENCE))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let Some(head) = read_head(&mut reader)? else { return Ok(()) };
    // A path that would reach outside the folder served is turned away:
    // `..` in an address must not reach the rest of the phone, and neither
    // must a link in the folder.
    let Some(target) = resolve(root, &head.path) else {
        return reply(&mut stream, 403, "text/plain; charset=utf-8", b"forbidden");
    };
    match head.method.as_str() {
        "GET" | "HEAD" => get(&mut stream, root, &head, &target),
        "PUT" => put(&mut stream, &mut reader, root, &head, &target),
        _ => reply(&mut stream, 405, "text/plain; charset=utf-8", b"not allowed"),
    }
}

/// The head of the request `reader` carries, or nothing where it carries
/// none.
fn read_head(reader: &mut BufReader<TcpStream>) -> Result<Option<Head>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let mut head = Head {
        method: parts.next().unwrap_or_default().to_uppercase(),
        path: parts.next().unwrap_or("/").to_string(),
        length: None,
        range: None,
    };
    let mut taken = line.len();
    loop {
        let mut header = String::new();
        let n = reader.read_line(&mut header)?;
        if n == 0 {
            break;
        }
        taken += n;
        if taken > HEAD_MAX {
            bail!("a request's head is longer than {HEAD_MAX} bytes");
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else { continue };
        let (name, value) = (name.trim(), value.trim());
        if name.eq_ignore_ascii_case("content-length") {
            head.length = value.parse().ok();
        } else if name.eq_ignore_ascii_case("range") {
            head.range = Some(value.to_string());
        }
    }
    Ok(Some(head))
}

/// Answer a download: the folder listed where `target` is one, the file
/// itself where it is a file, and the part of it asked for where a range
/// is.
fn get(stream: &mut TcpStream, root: &Path, head: &Head, target: &Path) -> Result<()> {
    let meta = match fs::metadata(target) {
        Ok(meta) => meta,
        Err(_) => return reply(stream, 404, "text/plain; charset=utf-8", b"not found"),
    };
    if !within(root, target) {
        return reply(stream, 403, "text/plain; charset=utf-8", b"forbidden");
    }
    if meta.is_dir() {
        let body = listing(root, target);
        let text = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(text.as_bytes())?;
        if head.method == "GET" {
            stream.write_all(body.as_bytes())?;
        }
        return Ok(());
    }
    if !meta.is_file() {
        return reply(stream, 404, "text/plain; charset=utf-8", b"not found");
    }
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let len = meta.len();
    let whole = (0u64, len.saturating_sub(1));
    let sought = head.range.as_deref().and_then(|range| parse_range(range, len));
    let (first, last) = sought.unwrap_or(whole);
    let length = if len == 0 { 0 } else { last - first + 1 };
    let mut text = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Disposition: {disposition}\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n",
        status = if sought.is_some() { 206 } else { 200 },
        reason = if sought.is_some() { "Partial Content" } else { "OK" },
        kind = kind_of(&name),
        disposition = disposition(&name),
    );
    if sought.is_some() {
        text.push_str(&format!("Content-Range: bytes {first}-{last}/{len}\r\n"));
    }
    text.push_str("\r\n");
    stream.write_all(text.as_bytes())?;
    if head.method != "GET" || length == 0 {
        return Ok(());
    }
    let mut file = fs::File::open(target).with_context(|| format!("open {}", target.display()))?;
    file.seek(SeekFrom::Start(first))?;
    let mut left = length;
    let mut buf = vec![0u8; CHUNK];
    while left > 0 {
        let want = left.min(CHUNK as u64) as usize;
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        stream.write_all(&buf[..n])?;
        left -= n as u64;
    }
    Ok(())
}

/// Answer an upload: the body, streamed to a file beside the one named and
/// then put in its place, so that a transfer that stops leaves no half of
/// a file behind under the name.
fn put(stream: &mut TcpStream, reader: &mut BufReader<TcpStream>, root: &Path, head: &Head, target: &Path) -> Result<()> {
    let Some(length) = head.length else {
        return reply(stream, 411, "text/plain; charset=utf-8", b"the length is not said");
    };
    if target.is_dir() {
        return reply(stream, 409, "text/plain; charset=utf-8", b"a folder is in the way");
    }
    let Some(parent) = target.parent() else {
        return reply(stream, 403, "text/plain; charset=utf-8", b"forbidden");
    };
    if fs::create_dir_all(parent).is_err() || !within(root, parent) {
        return reply(stream, 403, "text/plain; charset=utf-8", b"forbidden");
    }
    // A link where the file goes would be followed out of the folder served.
    if fs::symlink_metadata(target).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return reply(stream, 403, "text/plain; charset=utf-8", b"forbidden");
    }
    let mut beside = target.as_os_str().to_owned();
    beside.push(".part");
    let beside = PathBuf::from(beside);
    let written = (|| -> Result<()> {
        let mut file = fs::File::create(&beside).with_context(|| format!("make {}", beside.display()))?;
        let mut left = length;
        let mut buf = vec![0u8; CHUNK];
        while left > 0 {
            let want = left.min(CHUNK as u64) as usize;
            let n = reader.read(&mut buf[..want]).with_context(|| format!("read what is sent for {}", target.display()))?;
            if n == 0 {
                bail!("the upload ended after {} of {length} bytes", length - left);
            }
            file.write_all(&buf[..n])?;
            left -= n as u64;
        }
        file.sync_all()?;
        drop(file);
        fs::rename(&beside, target).with_context(|| format!("put {} in place", target.display()))
    })();
    match written {
        Ok(()) => reply(stream, 201, "text/plain; charset=utf-8", b"written"),
        Err(e) => {
            let _ = fs::remove_file(&beside);
            Err(e)
        }
    }
}

/// The answer `code` says, with `body`.
fn reply(stream: &mut TcpStream, code: u16, kind: &str, body: &[u8]) -> Result<()> {
    let reason = match code {
        201 => "Created",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        _ => "OK",
    };
    let text = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(text.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

/// Where the address `path` names is, under `root`; nothing where it would
/// reach outside it.
fn resolve(root: &Path, path: &str) -> Option<PathBuf> {
    let path = path.split(|c| c == '?' || c == '#').next().unwrap_or("/");
    let decoded = percent_decode(path);
    let mut out = root.to_path_buf();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            name => out.push(name),
        }
    }
    Some(out)
}

/// Whether `path`, with every link along the way followed, is inside
/// `root`.
fn within(root: &Path, path: &Path) -> bool {
    match path.canonicalize() {
        Ok(real) => real.starts_with(root),
        Err(_) => false,
    }
}

/// The first and last byte `header` asks for of a file of `len` bytes,
/// where it asks for a part that is there.
fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    if len == 0 {
        return None;
    }
    let asked = header.trim().strip_prefix("bytes=")?;
    let (from, to) = asked.split_once('-')?;
    let (first, last) = if from.trim().is_empty() {
        // The last `to` bytes, as a player asks for the end of a file.
        let tail: u64 = to.trim().parse().ok()?;
        (len.saturating_sub(tail), len - 1)
    } else {
        let first: u64 = from.trim().parse().ok()?;
        let last = match to.trim() {
            "" => len - 1,
            text => text.parse::<u64>().ok()?.min(len - 1),
        };
        (first, last)
    };
    (first <= last && first < len).then_some((first, last))
}

/// What a percent-encoded address says.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// The page that lists `dir`: a card for each entry, with the way up the
/// folder and the way to put files in it.
fn listing(root: &Path, dir: &Path) -> String {
    let shown = dir.strip_prefix(root).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let path_of = |name: &str| -> String { name.split('/').map(url_encode).collect::<Vec<_>>().join("/") };
    let prefix = if shown.is_empty() { "/".to_string() } else { format!("/{}/", path_of(&shown)) };
    let mut cards = String::new();
    if !shown.is_empty() {
        cards.push_str("<a class=card href=\"../\"><div class=face>&#8593;</div><div class=name>..</div><div class=note>上级</div></a>");
    }
    let mut entries: Vec<_> = fs::read_dir(dir).into_iter().flatten().filter_map(|entry| entry.ok()).collect();
    entries.sort_by_key(|entry| {
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        (!is_dir, entry.file_name().to_string_lossy().to_lowercase())
    });
    let mut count = 0;
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        let href = format!("{prefix}{}{}", url_encode(&name), if is_dir { "/" } else { "" });
        let (face, note) = if is_dir { ("&#128193;".to_string(), "文件夹".to_string()) } else { (face_of(&name), size_of(size)) };
        cards.push_str(&format!(
            "<a class=card href=\"{href}\"><div class=face>{face}</div><div class=name title=\"{title}\">{shown}</div><div class=note>{note}</div></a>",
            title = html_escape(&name),
            shown = html_escape(&name),
        ));
        count += 1;
    }
    if count == 0 {
        cards.push_str("<div class=empty>这个文件夹是空的</div>");
    }
    let mut crumbs = "<a href=\"/\">根目录</a>".to_string();
    if !shown.is_empty() {
        let mut walked = String::new();
        for part in shown.split('/').filter(|part| !part.is_empty()) {
            if !walked.is_empty() {
                walked.push('/');
            }
            walked.push_str(part);
            crumbs.push_str(&format!(" <span>/</span> <a href=\"/{}/\">{}</a>", path_of(&walked), html_escape(part)));
        }
    }
    format!(
        "<!doctype html><html lang=zh><head><meta charset=utf-8>\
<meta name=viewport content=\"width=device-width,initial-scale=1\">\
<title>Skyward 无线传输</title><style>{style}</style></head><body>\
<header><div class=bar><div class=brand><i></i>Skyward</div><div class=crumbs>{crumbs}</div>\
<button class=ghost onclick=\"pick()\">上传文件</button></div></header>\
<main><div class=grid>{cards}</div>\
<div id=drop onclick=\"pick()\"><b>把文件拖到这里，或点一下选择</b>放进当前文件夹</div>\
<div id=bar><i></i></div>\
<p class=hint>同一网络里的设备都能打开这一页，也能上传和下载。关掉无线传输，或离开恢复模式，它就消失。</p>\
</main><footer>共 {count} 项 · 由 Skyward 恢复模式提供</footer>\
<input id=file type=file multiple hidden><script>{script}</script></body></html>",
        style = STYLE,
        script = SCRIPT,
    )
}

const STYLE: &str = "
:root{--bg:#0f1115;--card:#181b22;--line:#262b36;--text:#e8ebf1;--dim:#8b93a3;--accent:#4d8dff}
@media (prefers-color-scheme:light){:root{--bg:#f5f6f8;--card:#fff;--line:#e3e6ec;--text:#15181e;--dim:#6b7280;--accent:#2563eb}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font:15px/1.6 -apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,sans-serif}
header{position:sticky;top:0;z-index:2;background:var(--bg);border-bottom:1px solid var(--line)}
.bar{max-width:1080px;margin:0 auto;padding:14px 20px;display:flex;align-items:center;gap:14px;flex-wrap:wrap}
.brand{display:flex;align-items:center;gap:9px;font-weight:700;white-space:nowrap}
.brand i{width:9px;height:9px;border-radius:50%;background:#3ddc84;box-shadow:0 0 8px #3ddc84}
.crumbs{flex:1;min-width:180px;font-size:14px;color:var(--dim);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.crumbs a{color:var(--dim);text-decoration:none}.crumbs a:hover{color:var(--accent)}
.crumbs span{opacity:.4;margin:0 4px}
button{padding:8px 16px;border:0;border-radius:9px;background:var(--accent);color:#fff;font-size:14px;font-weight:600;cursor:pointer}
button.ghost{background:transparent;border:1px solid var(--line);color:var(--text)}
button.ghost:hover{border-color:var(--accent);color:var(--accent)}
main{max-width:1080px;margin:0 auto;padding:20px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:12px}
.card{display:flex;flex-direction:column;align-items:center;gap:6px;padding:16px 10px;background:var(--card);border:1px solid transparent;border-radius:14px;text-decoration:none;color:inherit}
.card:hover{border-color:var(--line);transform:translateY(-1px)}
.card .face{font-size:30px;line-height:1}
.card .name{width:100%;font-size:13px;text-align:center;word-break:break-all;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}
.card .note{font-size:11px;color:var(--dim)}
.empty{grid-column:1/-1;padding:56px 0;text-align:center;color:var(--dim)}
#drop{margin-top:18px;padding:30px;border:2px dashed var(--line);border-radius:16px;text-align:center;color:var(--dim);cursor:pointer}
#drop.over{border-color:var(--accent);color:var(--accent)}
#drop b{display:block;color:var(--text);margin-bottom:4px}
#bar{margin-top:12px;height:6px;border-radius:3px;background:var(--line);overflow:hidden;display:none}
#bar i{display:block;height:100%;width:0;background:var(--accent);transition:width .2s}
.hint{margin:18px 0 0;color:var(--dim);font-size:12px}
footer{max-width:1080px;margin:0 auto;padding:18px 20px 36px;color:var(--dim);font-size:12px;text-align:center}
";

const SCRIPT: &str = "
const input=document.getElementById('file'),drop=document.getElementById('drop'),bar=document.getElementById('bar'),fill=bar.querySelector('i');
const base=location.pathname.endsWith('/')?location.pathname:location.pathname+'/';
function pick(){input.click()}
input.onchange=()=>{if(input.files.length)up([...input.files]);input.value=''};
['dragenter','dragover'].forEach(e=>drop.addEventListener(e,ev=>{ev.preventDefault();drop.classList.add('over')}));
['dragleave','drop'].forEach(e=>drop.addEventListener(e,ev=>{ev.preventDefault();drop.classList.remove('over')}));
drop.addEventListener('drop',ev=>{if(ev.dataTransfer.files.length)up([...ev.dataTransfer.files])});
async function up(files){
  bar.style.display='block';
  for(let i=0;i<files.length;i++){
    const f=files[i];
    await new Promise((done,fail)=>{
      const x=new XMLHttpRequest();
      x.open('PUT',base+encodeURIComponent(f.name));
      x.upload.onprogress=e=>{if(e.lengthComputable)fill.style.width=((i+e.loaded/e.total)/files.length*100).toFixed(0)+'%'};
      x.onload=()=>x.status<400?done():fail(new Error(x.status));
      x.onerror=()=>fail(new Error('the network broke'));
      x.send(f);
    }).catch(()=>{});
  }
  fill.style.width='100%';
  location.reload();
}
";

/// What a browser is told a file is, by what its name ends with.
fn kind_of(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().filter(|ext| *ext != name).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "txt" | "log" | "md" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "xml" => "application/xml",
        "csv" => "text/csv; charset=utf-8",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "apk" => "application/vnd.android.package-archive",
        "img" => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

/// Whether a browser shows a file in place or saves it, by what it is.
fn inline(name: &str) -> bool {
    matches!(kind_of(name).split('/').next(), Some("image" | "video" | "audio")) || kind_of(name).starts_with("text/plain")
}

/// How a browser is told the file's name, plain where it can be and in
/// UTF-8's own form where it cannot.
fn disposition(name: &str) -> String {
    let plain: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
    let encoded: String = name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    let kind = if inline(name) { "inline" } else { "attachment" };
    format!("{kind}; filename=\"{plain}\"; filename*=UTF-8''{encoded}")
}

/// A picture for a name, by what it ends with.
fn face_of(name: &str) -> String {
    let ext = name.rsplit('.').next().filter(|ext| *ext != name).unwrap_or("").to_lowercase();
    let face = match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" | "svg" => "&#128444;",
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "3gp" => "&#127909;",
        "mp3" | "flac" | "wav" | "m4a" | "ogg" | "aac" => "&#127925;",
        "zip" | "7z" | "tar" | "gz" | "xz" | "zst" | "rar" | "bz2" => "&#128230;",
        "apk" | "apks" => "&#128241;",
        "img" | "bin" | "lz4" => "&#128190;",
        "txt" | "md" | "log" => "&#128196;",
        "pdf" => "&#128209;",
        "json" | "xml" | "csv" => "&#128202;",
        _ => "&#128196;",
    };
    face.to_string()
}

fn size_of(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let size = bytes as f64;
    if size >= K * K * K {
        format!("{:.1} GB", size / (K * K * K))
    } else if size >= K * K {
        format!("{:.1} MB", size / (K * K))
    } else if size >= K {
        format!("{:.0} KB", size / K)
    } else {
        format!("{bytes} B")
    }
}

fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("skyward-serve-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// One request, answered by `answer` from a thread of its own, and what
    /// came back.
    fn ask(root: &Path, request: &[u8]) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = root.to_path_buf();
        let thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = answer(stream, &served);
        });
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        client.set_write_timeout(Some(Duration::from_secs(20))).unwrap();
        client.write_all(request).unwrap();
        let mut said = Vec::new();
        client.read_to_end(&mut said).unwrap();
        thread.join().unwrap();
        String::from_utf8_lossy(&said).into_owned()
    }

    #[test]
    fn an_address_that_would_reach_outside_the_folder_is_refused() {
        let root = Path::new("/sdcard");
        assert_eq!(resolve(root, "/DCIM/a.jpg").unwrap(), Path::new("/sdcard/DCIM/a.jpg"));
        assert_eq!(resolve(root, "/a%20b.txt").unwrap(), Path::new("/sdcard/a b.txt"));
        assert_eq!(resolve(root, "/DCIM/?sort=name").unwrap(), Path::new("/sdcard/DCIM"));
        assert_eq!(resolve(root, "/../data"), None, "a way out of the folder served");
        assert_eq!(resolve(root, "/DCIM/../../data"), None);
    }

    #[test]
    fn a_range_asks_for_the_part_of_a_file_it_names() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)), "the last hundred");
        assert_eq!(parse_range("bytes=900-5000", 1000), Some((900, 999)), "past the end, to the end");
        assert_eq!(parse_range("bytes=1000-", 1000), None, "past the end altogether");
        assert_eq!(parse_range("bytes=0-0", 0), None, "nothing to take");
        assert_eq!(parse_range("items=0-1", 1000), None, "not bytes");
    }

    #[test]
    fn a_file_is_taken_with_the_part_asked_for() {
        let root = scratch("get");
        fs::write(root.join("hello.txt"), b"hello").unwrap();
        let said = ask(&root, b"GET /hello.txt HTTP/1.1\r\nHost: phone\r\nRange: bytes=1-3\r\n\r\n");
        assert!(said.starts_with("HTTP/1.1 206 Partial Content"), "{said}");
        assert!(said.contains("Content-Range: bytes 1-3/5"), "{said}");
        assert!(said.ends_with("\r\n\r\nell"), "{said}");
        let whole = ask(&root, b"GET /hello.txt HTTP/1.1\r\nHost: phone\r\n\r\n");
        assert!(whole.starts_with("HTTP/1.1 200 OK"), "{whole}");
        assert!(whole.ends_with("\r\n\r\nhello"), "{whole}");
    }

    #[test]
    fn a_file_put_there_is_written_and_a_link_out_is_not() {
        let root = scratch("put");
        let said = ask(&root, b"PUT /DCIM/new.txt HTTP/1.1\r\nHost: phone\r\nContent-Length: 5\r\n\r\nhallo");
        assert!(said.starts_with("HTTP/1.1 201 Created"), "{said}");
        assert_eq!(fs::read(root.join("DCIM/new.txt")).unwrap(), b"hallo");
        assert!(!root.join("DCIM/new.txt.part").exists(), "no half is left behind");

        let out = std::env::temp_dir().join(format!("skyward-serve-out-{}", std::process::id()));
        fs::create_dir_all(&out).unwrap();
        std::os::unix::fs::symlink(&out, root.join("escape")).unwrap();
        let refused = ask(&root, b"PUT /escape/taken.txt HTTP/1.1\r\nContent-Length: 2\r\n\r\nno");
        assert!(refused.starts_with("HTTP/1.1 403 Forbidden"), "a link is not a way out: {refused}");
        assert!(!out.join("taken.txt").exists());
    }

    #[test]
    fn a_folder_is_listed_with_what_a_browser_needs_to_escape() {
        let root = scratch("list");
        fs::create_dir_all(root.join("DCIM")).unwrap();
        fs::write(root.join("a b&c.txt"), b"x").unwrap();
        let page = listing(&root, &root);
        assert!(page.contains("href=\"/DCIM/\""), "{page}");
        assert!(page.contains("href=\"/a%20b%26c.txt\""), "the address is escaped: {page}");
        assert!(page.contains("a b&amp;c.txt"), "the name is escaped: {page}");
        assert!(page.contains("共 2 项"), "{page}");
    }

    #[test]
    fn the_whole_folder_is_served_on_a_port_of_its_own() {
        let root = scratch("server");
        fs::write(root.join("one.txt"), b"one").unwrap();
        let server = Server::start_at(root.clone(), 0).unwrap();
        let url = format!("http://127.0.0.1:{}/one.txt", server.port());
        let mut client = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
        client.write_all(format!("GET /one.txt HTTP/1.1\r\nHost: {url}\r\n\r\n").as_bytes()).unwrap();
        let mut said = Vec::new();
        client.read_to_end(&mut said).unwrap();
        let said = String::from_utf8_lossy(&said);
        assert!(said.ends_with("\r\n\r\none"), "{said}");
        // The request is answered on a thread of its own, so the count is read
        // once it has been.
        for _ in 0..50 {
            if server.served() == 1 {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(server.served(), 1);
    }
}
