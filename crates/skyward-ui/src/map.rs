//! The phone's storage as a map.
//!
//! Each partition of the system's own is a piece of ground, its area the
//! room it takes: `system` is a country, `mi_ext` a garden. The partitions
//! that are not the system's are said in a list under the map instead,
//! because `super` is nine tenths of everything that is not the data
//! partition, and drawn to scale the rest would be a line of thread.
//!
//! The data partition is that same nine tenths of the whole phone, and is
//! said in numbers above the map.

use crate::canvas::Canvas;
use crate::font::Fonts;
use crate::geom::Rect;
use crate::lang::Lang;
use crate::theme::Theme;
use crate::Rgba;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

/// Where the phone's partitions are named, and where the logical ones stand
/// once the boot has mapped them out of `super`.
const BY_NAME: &str = "/dev/block/by-name";
const MAPPER: &str = "/dev/block/mapper";

/// How many partitions that are not the system's the list says. A phone has
/// some fifty of them and the page has room for twelve; the smallest are
/// counted, not named.
const LISTED: usize = 12;

/// What a piece of the phone is for, and so what it is drawn in.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Tone {
    /// The system's own: `system`, `vendor`, `product`, `system_ext`, `odm`.
    System,
    /// The firmware a radio and a bootloader run from.
    Firmware,
    /// What is written to start the phone.
    Boot,
    /// Where the phone keeps what it has: the data partition and its
    /// bookkeeping.
    Storage,
    /// Room `super` has not given out yet, and the small things the phone
    /// needs and never looks at.
    #[default]
    Errand,
}

impl Tone {
    /// The colour the stock Settings fills this kind of icon's tile with.
    fn ink(self) -> Rgba {
        match self {
            Tone::System => Rgba::rgb(0x34, 0x82, 0xff),
            Tone::Firmware => Rgba::rgb(0x77, 0x67, 0xf9),
            Tone::Boot => Rgba::rgb(0xff, 0x69, 0x00),
            Tone::Storage => Rgba::rgb(0x1d, 0xcd, 0x3a),
            Tone::Errand => Rgba::rgb(0x8c, 0x9d, 0xb0),
        }
    }

    fn said(self, l: Lang) -> &'static str {
        match self {
            Tone::System => l.pick(["系统", "系統", "System"]),
            Tone::Firmware => l.pick(["固件", "韌體", "Firmware"]),
            Tone::Boot => l.pick(["启动", "啟動", "Boot"]),
            Tone::Storage => l.pick(["数据", "資料", "Data"]),
            Tone::Errand => l.pick(["其它", "其他", "Other"]),
        }
    }

    const ALL: [Tone; 5] = [Tone::System, Tone::Firmware, Tone::Boot, Tone::Storage, Tone::Errand];
}

/// A piece of the phone: the room it takes, what it is for, where it is, and
/// what is there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub bytes: u64,
    pub tone: Tone,
    /// The partitions a holding partition carries, drawn standing on it.
    pub inside: Vec<Part>,
    /// Whether the piece is a partition of the phone. The room `super` has
    /// not given out is drawn, and is not one.
    pub counted: bool,
    /// The block device the partition is, as the phone names it.
    pub device: Option<String>,
    /// The filesystem on it, where one is mounted from it.
    pub filesystem: Option<String>,
    /// Where it is mounted, where it is.
    pub at: Option<String>,
}

impl Part {
    pub fn new(name: impl Into<String>, bytes: u64, tone: Tone) -> Self {
        Part { name: name.into(), bytes, tone, counted: true, ..Part::default() }
    }

    /// A partition that holds others: its room is theirs, unless `bytes`
    /// says it is larger than what stands on it.
    pub fn holding(name: impl Into<String>, bytes: u64, tone: Tone, inside: Vec<Part>) -> Self {
        Part { inside, ..Part::new(name, bytes, tone) }
    }

    /// The room a holding partition has not given out, drawn so that what
    /// would fit in it is visible.
    pub fn spare(bytes: u64) -> Self {
        Part { counted: false, ..Part::new("free", bytes, Tone::Errand) }
    }

    pub fn filled(&self) -> u64 {
        self.inside.iter().map(Part::filled).sum::<u64>().max(self.bytes)
    }

    fn parts(&self) -> usize {
        self.inside.iter().map(Part::parts).sum::<usize>() + usize::from(self.counted)
    }
}

/// The phone's partitions as one page shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    /// `super`, with the system's own standing on it.
    pub holding: Option<Part>,
    /// The partitions that are not the system's, largest first.
    pub others: Vec<Part>,
    /// How many of those there were before the list was cut to [`LISTED`].
    pub dropped: usize,
    /// The data partition, said in numbers rather than drawn.
    pub data: Option<Part>,
    /// How much of the data partition is taken.
    pub used: Option<u64>,
}

impl Table {
    /// Everything the map draws, in the order it draws it: what stands on
    /// `super`, then the list. A tap on a rect is a tap on this.
    pub fn parts(&self) -> Vec<&Part> {
        let inside = self.holding.as_ref().map(|h| h.inside.iter()).into_iter().flatten();
        inside.chain(self.others.iter()).collect()
    }

    /// The room the system and its firmware take together.
    pub fn system(&self) -> u64 {
        self.holding.as_ref().map_or(0, Part::filled) + self.others.iter().map(Part::filled).sum::<u64>()
    }

    /// The partitions the page counts: the data partition, what stands on
    /// `super`, and the list.
    pub fn count(&self) -> usize {
        self.holding.as_ref().map_or(0, Part::parts) + self.others.len() + usize::from(self.data.is_some())
    }

    /// What the phone holds, as read once from the running recovery. A
    /// second call answers what the first did, the table not changing under
    /// the pages.
    pub fn of_this_phone() -> &'static Table {
        static TABLE: OnceLock<Table> = OnceLock::new();
        TABLE.get_or_init(read)
    }
}

/// Read the partition table from the phone's own names for them.
fn read() -> Table {
    let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut table = Table::default();
    let mut others = Vec::new();
    for (name, device, bytes) in entries(Path::new(BY_NAME)) {
        if bytes == 0 || (name.ends_with("_b") && has_other_slot(&name)) {
            continue;
        }
        let mut part = Part::new(name.clone(), bytes, tone_of(&name));
        part.device = Some(device.clone());
        if let Some((at, filesystem)) = mounted(&mounts, &device) {
            part.at = Some(at);
            part.filesystem = Some(filesystem);
        }
        if name == "userdata" {
            table.data = Some(part);
        } else if name == "super" {
            let mut inside = sorted(
                entries(Path::new(MAPPER))
                    .into_iter()
                    .filter(|(_, _, bytes)| *bytes > 0)
                    .map(|(name, device, bytes)| {
                        let tone = tone_of(&name);
                        Part { device: Some(device), ..Part::new(name, bytes, tone) }
                    })
                    .collect::<Vec<_>>(),
            );
            // What `super` has not given out: the room a package of a whole
            // system would go in.
            let held: u64 = inside.iter().map(|p| p.bytes).sum();
            if bytes > held {
                inside.push(Part::spare(bytes - held));
            }
            table.holding = Some(Part::holding(name, bytes.max(held), part.tone, inside));
        } else {
            others.push(part);
        }
    }
    others.sort_by_key(|p| std::cmp::Reverse(p.bytes));
    table.dropped = others.len().saturating_sub(LISTED);
    others.truncate(LISTED);
    table.others = others;
    if let Some(data) = table.data.as_ref() {
        table.used = data.at.as_deref().and_then(taken_of);
    }
    table
}

fn sorted(mut parts: Vec<Part>) -> Vec<Part> {
    parts.sort_by_key(|p| std::cmp::Reverse(p.bytes));
    parts
}

/// Whether the phone names both slots' copy of `name`, in which case only
/// the one the boot runs from is worth a place on the page.
fn has_other_slot(name: &str) -> bool {
    let other = format!("{}_a", name.trim_end_matches("_b"));
    Path::new(BY_NAME).join(other).exists()
}

/// The partitions under `dir`: their name, the block device behind it, and
/// how many bytes it is.
fn entries(dir: &Path) -> Vec<(String, String, u64)> {
    let Ok(list) = fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in list.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(target) = fs::read_link(entry.path()) else { continue };
        let Some(node) = target.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        out.push((name, dir.join(&target).display().to_string(), bytes_of(&node)));
    }
    out
}

/// How many bytes a block device is, as the kernel counts them: sectors of
/// 512.
fn bytes_of(node: &str) -> u64 {
    fs::read_to_string(format!("/sys/class/block/{node}/size"))
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .map_or(0, |sectors| sectors * 512)
}

/// Where a block device is mounted, and with what filesystem, from the mount
/// table the running phone keeps.
fn mounted(mounts: &str, device: &str) -> Option<(String, String)> {
    let node = device.rsplit('/').next().unwrap_or(device);
    mounts.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let (from, at, filesystem) = (fields.next()?, fields.next()?, fields.next()?);
        let same = from == device || from.rsplit('/').next() == Some(node);
        same.then(|| (at.to_string(), filesystem.to_string()))
    })
}

/// How much of the partition at `at` is taken, as the filesystem there
/// counts it. `None` where it is not mounted, or the count cannot be read.
#[cfg(target_os = "android")]
fn taken_of(at: &str) -> Option<u64> {
    let path = std::ffi::CString::new(at).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut s) } != 0 {
        return None;
    }
    // `fsblkcnt_t` is 32 bits on some hosts and 64 bits on Android.
    #[allow(clippy::useless_conversion)]
    let (blocks, free) = (u64::from(s.f_blocks), u64::from(s.f_bfree));
    Some(blocks.saturating_sub(free).saturating_mul(s.f_frsize))
}

/// A computer drawing the pages has no phone's filesystem to ask, and the
/// pages say `—` there.
#[cfg(not(target_os = "android"))]
fn taken_of(_at: &str) -> Option<u64> {
    None
}

/// What a partition is for, from the name the phone gives it.
fn tone_of(name: &str) -> Tone {
    let stem = name.trim_end_matches("_a").trim_end_matches("_b");
    match stem {
        "userdata" | "metadata" | "persist" | "cache" => Tone::Storage,
        "system" | "vendor" | "product" | "system_ext" | "odm" | "mi_ext" | "vendor_dlkm" | "odm_dlkm" | "system_dlkm"
        | "cust" | "rescue" => Tone::System,
        "modem" | "modemst1" | "modemst2" | "fsg" | "fsc" | "xbl" | "xbl_config" | "abl" | "devcfg" | "devcfg_b"
        | "qupfw" | "uefisecapp" | "tz" | "hyp" | "keymaster" | "cmnlib" | "cmnlib64" => Tone::Firmware,
        "boot" | "init_boot" | "dtbo" | "vendor_boot" | "recovery" | "vbmeta" | "vbmeta_system" | "vbmeta_vendor" => Tone::Boot,
        _ => Tone::Errand,
    }
}

/// How much room a piece takes, as the piece itself would say it: gigabytes
/// to one decimal where it is that large, megabytes below.
pub fn size(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 {
        return if gb >= 10.0 { format!("{gb:.0}GB") } else { format!("{gb:.1}GB") };
    }
    let mb = bytes as f64 / 1e6;
    if mb >= 10.0 {
        return format!("{mb:.0}MB");
    }
    if mb >= 1.0 {
        return format!("{mb:.1}MB");
    }
    format!("{}KB", (bytes as f64 / 1e3).max(0.1).round() as u64)
}

/// Draw the whole page: what the phone is made of in numbers, the map of the
/// room the system takes, the partitions that are not the system's, and what
/// the colours mean. What comes back is a rect for each of [`Table::parts`],
/// in that order, so that a tap can say which.
pub fn page(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, table: &Table) -> Vec<Rect> {
    let dp = |v: f32| t.m.dp(v);
    let margin = t.m.margin;
    let wide = t.m.w as f32 - margin * 2.0;
    let data = table.data.as_ref().map_or(0, |d| d.bytes);

    c.text(&f.title, l.pick(["分区地图", "分區地圖", "Storage map"]), margin, dp(86.0), dp(30.0), t.pal.text, Some(wide));
    let said = format!("{} · {} {}", size(data + table.system()), table.count(), l.pick(["个分区", "個分區", "partitions"]));
    c.text(&f.body, &said, margin, dp(112.0), dp(13.0), t.pal.subtitle, Some(wide));

    // Three numbers: the whole, what the files have, and what the system
    // takes. The map below is the third of them.
    let mut x = margin;
    for (value, label) in [
        (if table.data.is_some() { size(data) } else { "—".to_string() }, l.pick(["数据分区", "資料分區", "Data"])),
        (size(table.system()), l.pick(["系统与固件", "系統與韌體", "System"])),
        (
            match table.used {
                Some(u) => size(u),
                None => "—".to_string(),
            },
            l.pick(["data 已用", "data 已用", "Used"]),
        ),
    ] {
        c.text(&f.heading, &value, x, dp(160.0), dp(23.0), t.pal.text, Some(wide / 3.0));
        c.text(&f.body, label, x, dp(180.0), dp(12.0), t.pal.subtitle, Some(wide / 3.0));
        x += wide / 3.0;
    }

    let mut spots = Vec::new();
    // The system's own, drawn: the ground it stands on is a card, and each
    // partition a piece of it.
    let map = Rect::new(margin, dp(234.0), wide, dp(440.0));
    match table.holding.as_ref().filter(|h| !h.inside.is_empty()) {
        Some(holding) => {
            let label = format!("{} · {}", holding.name, size(holding.filled()));
            c.text(&f.label, &label, margin, dp(222.0), dp(12.0), t.pal.subtitle, Some(wide));
            c.fill_round_rect(map.x, map.y, map.w, map.h, dp(16.0), t.pal.card);
            let inner = Rect::new(map.x + dp(10.0), map.y + dp(10.0), map.w - dp(20.0), map.h - dp(20.0));
            spots.extend(tiles(c, t, f, l, inner, &holding.inside));
        }
        None => {
            let note = l.pick([
                "没读到 super 的分区表：这一版的 recovery 只画它自己读得出来的分区。",
                "沒讀到 super 的分區表：這一版的 recovery 只畫它自己讀得出來的分區。",
                "The partitions inside super were not read: the recovery draws what it can read of them.",
            ]);
            c.text(&f.body, note, margin, dp(260.0), dp(13.0), t.pal.subtitle, Some(wide));
        }
    }

    // What is not the system's, said rather than drawn: a few tens of
    // megabytes each against a gigabyte of system.
    let top = map.y + map.h + dp(34.0);
    c.text(&f.label, l.pick(["其它分区", "其它分區", "Other partitions"]), margin, top, dp(12.0), t.pal.subtitle, Some(wide));
    let cell = wide / 2.0;
    let row = dp(38.0);
    for (i, part) in table.others.iter().enumerate() {
        let (col, line) = (i % 2, i / 2);
        let x = margin + col as f32 * cell;
        let y = top + dp(14.0) + line as f32 * row;
        c.fill_circle(x + dp(4.0), y + dp(9.0), dp(3.5), part.tone.ink().with_alpha(if t.dark { 235 } else { 210 }));
        c.text(&f.heading, &part.name, x + dp(14.0), y + dp(13.5), dp(13.0), t.pal.text, Some(cell - dp(90.0)));
        let said = size(part.bytes);
        let room = c.text_width(&f.body, &said, dp(12.5));
        c.text(&f.body, &said, x + cell - dp(12.0) - room, y + dp(13.5), dp(12.5), t.pal.value, None);
        spots.push(Rect::new(x, y - dp(4.0), cell, row));
    }

    if table.dropped > 0 {
        let said = format!(
            "{} {} {}",
            l.pick(["另有", "另有", "and"]),
            table.dropped,
            l.pick(["个更小的分区没有画出。", "個更小的分區沒有畫出。", "smaller partitions are not drawn."]),
        );
        c.text(&f.body, &said, margin, top + dp(14.0) + table.others.len().div_ceil(2) as f32 * row + dp(6.0), dp(12.0), t.pal.subtitle, Some(wide));
    }

    // What the colours mean, in a row under the list.
    let base = t.m.h as f32 - dp(40.0);
    let mut x = margin;
    for tone in Tone::ALL {
        c.fill_circle(x + dp(4.0), base - dp(4.0), dp(4.0), tone.ink().with_alpha(if t.dark { 235 } else { 210 }));
        let said = tone.said(l);
        let room = c.text_width(&f.body, said, dp(12.5));
        c.text(&f.body, said, x + dp(13.0), base, dp(12.5), t.pal.subtitle, None);
        x += dp(13.0) + room + dp(16.0);
    }
    spots
}

/// Draw one partition's own page: what it is, how big, where it lives.
pub fn detail(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, part: &Part) {
    let dp = |v: f32| t.m.dp(v);
    let margin = t.m.margin;
    let wide = t.m.w as f32 - margin * 2.0;
    let top = t.m.content_top;

    c.fill_round_rect(margin, top, wide, dp(120.0), t.m.card_radius, t.pal.card);
    c.fill_round_rect(margin + dp(18.0), top + dp(20.0), dp(6.0), dp(80.0), dp(3.0), part.tone.ink());
    c.text(&f.heading, &part.name, margin + dp(38.0), top + dp(46.0), dp(22.0), t.pal.text, Some(wide - dp(60.0)));
    let said = size(part.bytes);
    c.text(&f.body, &said, margin + dp(38.0), top + dp(72.0), dp(15.0), t.pal.subtitle, Some(wide - dp(60.0)));
    c.text(&f.body, part.tone.said(l), margin + dp(38.0), top + dp(96.0), dp(13.0), t.pal.subtitle, Some(wide - dp(60.0)));

    let mut rows: Vec<(&str, String)> = Vec::new();
    if let Some(device) = &part.device {
        rows.push((l.pick(["设备", "裝置", "Device"]), device.clone()));
    }
    if let Some(filesystem) = &part.filesystem {
        rows.push((l.pick(["文件系统", "檔案系統", "Filesystem"]), filesystem.clone()));
    }
    if let Some(at) = &part.at {
        rows.push((l.pick(["挂载于", "掛載於", "Mounted at"]), at.clone()));
    }
    if part.counted && !part.inside.is_empty() {
        let said = format!("{} · {}", part.inside.len(), l.pick(["个分区", "個分區", "partitions"]));
        rows.push((l.pick(["包含", "包含", "Holds"]), said));
    }
    if rows.is_empty() {
        rows.push((l.pick(["备注", "備註", "Note"]), l.pick(["这台机器没有说明这个分区。", "這台機器沒有說明這個分區。", "Nothing is said about this partition."]).to_string()));
    }

    let mut y = top + dp(136.0);
    let row_h = dp(56.0);
    c.fill_round_rect(margin, y, wide, row_h * rows.len() as f32 + dp(8.0), t.m.card_radius, t.pal.card);
    for (label, value) in rows {
        let inner = y + dp(10.0);
        c.text(&f.body, label, margin + dp(18.0), inner + dp(18.0), dp(13.0), t.pal.subtitle, Some(wide * 0.34));
        c.text(&f.heading, &value, margin + dp(18.0) + wide * 0.36, inner + dp(18.0), dp(14.0), t.pal.text, Some(wide * 0.6));
        y += row_h;
    }
}

/// Each piece of ground, and where it was drawn.
fn tiles(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, area: Rect, parts: &[Part]) -> Vec<Rect> {
    let dp = |v: f32| t.m.dp(v);
    let placed = room(area, parts);
    for (part, rect) in parts.iter().zip(&placed) {
        // The room a partition has not given out is a shadow of a tile, not
        // a tile: a package that would fit there is what the map is read for.
        // On a page whose card is white the ground has to be laid darker
        // than on a dark one, or it and the card are the same white.
        let (fill, edge) = if part.counted {
            (part.tone.ink().with_alpha(if t.dark { 54 } else { 92 }), part.tone.ink().with_alpha(if t.dark { 96 } else { 84 }))
        } else {
            (part.tone.ink().with_alpha(if t.dark { 30 } else { 44 }), part.tone.ink().with_alpha(if t.dark { 60 } else { 56 }))
        };
        c.fill_round_rect(rect.x, rect.y, rect.w, rect.h, dp(10.0), fill);
        c.stroke_round_rect(rect.x, rect.y, rect.w, rect.h, dp(10.0), dp(0.9), edge);
        name(c, t, f, l, part, *rect);
    }
    placed
}

/// The name of a piece, and its size where there is room for both. A name
/// too wide for the tile is broken after an underscore and written on two
/// lines — `system_` over `ext` — and only where that fails is the colour
/// left to speak alone.
fn name(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, part: &Part, rect: Rect) {
    let dp = |v: f32| t.m.dp(v);
    let pad = dp(11.0);
    let room = rect.w - pad * 2.0;
    if room <= dp(28.0) {
        return;
    }
    let said = if part.counted { part.name.clone() } else { l.pick(["空闲", "空閒", "Free"]).to_string() };
    let both = rect.h >= dp(58.0) && room >= dp(64.0);
    let tall = rect.h - pad * 2.0;
    // The room asked for is a shade under what there is: a drawn run is
    // measured a little wider than the sum of its glyphs.
    let fits = |c: &Canvas, px: f32| c.text_width(&f.heading, &said, px) <= room - dp(4.0);

    let mut px = if both { dp(15.0) } else { dp(12.0) };
    let mut lines = vec![said.clone()];
    if !fits(c, px) {
        match broken(c, f, &said, px, room - dp(4.0)) {
            Some(ls) if ls.len() > 1 && ls.len() as f32 * px * 1.3 <= tall => lines = ls,
            _ => {
                px *= 0.82;
                if !fits(c, px) {
                    return;
                }
            }
        }
    }
    for (i, line) in lines.iter().enumerate() {
        let base = rect.y + pad + px * 0.8 + i as f32 * px * 1.3;
        if base > rect.y + rect.h - dp(2.0) {
            break;
        }
        c.text(&f.heading, line, rect.x + pad, base, px, t.pal.text, Some(room));
    }
    if both && lines.len() == 1 {
        let size = size(part.bytes);
        c.text(&f.body, &size, rect.x + pad, rect.y + pad + px * 2.1, dp(12.0), t.pal.subtitle, Some(room));
    }
}

/// `name` broken after its underscores into the fewest lines that fit
/// `room`: `vendor_` over `dlkm` rather than `vendor…`. `None` where a
/// piece of the name is already wider than the room.
fn broken(c: &Canvas, f: &Fonts, name: &str, px: f32, room: f32) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (i, ch) in name.char_indices() {
        if ch == '_' {
            parts.push(&name[start..=i]);
            start = i + 1;
        }
    }
    if start < name.len() {
        parts.push(&name[start..]);
    }
    let mut lines: Vec<String> = Vec::new();
    let mut now = String::new();
    for part in parts {
        let tried = format!("{now}{part}");
        if c.text_width(&f.heading, &tried, px) <= room {
            now = tried;
            continue;
        }
        if c.text_width(&f.heading, part, px) > room {
            return None;
        }
        if !now.is_empty() {
            lines.push(std::mem::take(&mut now));
        }
        now = part.to_string();
    }
    if !now.is_empty() {
        lines.push(now);
    }
    Some(lines)
}

/// Where each piece of ground goes: the area cut up in proportion to what
/// each takes, the longer side cut first so that none is a sliver.
fn room(area: Rect, parts: &[Part]) -> Vec<Rect> {
    let weights: Vec<f32> = parts.iter().map(|p| p.filled().max(1) as f32).collect();
    let mut out = vec![Rect::new(0.0, 0.0, 0.0, 0.0); parts.len()];
    cut(area, &weights, 0, &mut out);
    let gap = 3.0 * (area.w / 1000.0).clamp(1.0, 3.0);
    out.iter()
        .map(|r| Rect::new(r.x + gap, r.y + gap, (r.w - gap * 2.0).max(0.0), (r.h - gap * 2.0).max(0.0)))
        .collect()
}

/// Cut `area` among `weights` from `at`, in two halves and each of those
/// again, the longer side first.
fn cut(area: Rect, weights: &[f32], at: usize, out: &mut [Rect]) {
    if weights.is_empty() {
        return;
    }
    if weights.len() == 1 {
        out[at] = area;
        return;
    }
    let total: f32 = weights.iter().sum();
    // The cut that leaves the two halves as near equal as the weights allow.
    let mut best = (f32::MAX, 1);
    let mut held = 0.0;
    for (i, w) in weights.iter().enumerate().take(weights.len() - 1) {
        held += w;
        let off = (held / total - 0.5).abs();
        if off < best.0 {
            best = (off, i + 1);
        }
    }
    let (k, share) = (best.1, weights.iter().take(best.1).sum::<f32>() / total);
    let (first, second) = if area.w >= area.h {
        let w = area.w * share;
        (Rect::new(area.x, area.y, w, area.h), Rect::new(area.x + w, area.y, area.w - w, area.h))
    } else {
        let h = area.h * share;
        (Rect::new(area.x, area.y, area.w, h), Rect::new(area.x, area.y + h, area.w, area.h - h))
    };
    cut(first, &weights[..k], at, out);
    cut(second, &weights[k..], at + k, out);
}
