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

/// What a piece of the phone is for, and so what it is drawn in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// A piece of the phone: the room it takes, what it is for, and, where it
/// holds others, what stands on it.
pub struct Tile<'a> {
    pub name: &'a str,
    pub bytes: u64,
    pub tone: Tone,
    pub inside: Vec<Tile<'a>>,
    /// Whether the piece is a partition of the phone. The room `super` has
    /// not given out is drawn, and is not one.
    counted: bool,
}

impl<'a> Tile<'a> {
    pub fn new(name: &'a str, bytes: u64, tone: Tone) -> Self {
        Tile { name, bytes, tone, inside: Vec::new(), counted: true }
    }

    /// A partition that holds others: its room is theirs, unless `bytes`
    /// says it is larger than what stands on it.
    pub fn holding(name: &'a str, bytes: u64, tone: Tone, inside: Vec<Tile<'a>>) -> Self {
        Tile { name, bytes, tone, inside, counted: true }
    }

    /// The room a holding partition has not given out, drawn so that what
    /// would fit in it is visible.
    pub fn spare(bytes: u64) -> Self {
        Tile { name: "free", bytes, tone: Tone::Errand, inside: Vec::new(), counted: false }
    }

    fn filled(&self) -> u64 {
        self.inside.iter().map(Tile::filled).sum::<u64>().max(self.bytes)
    }

    fn parts(&self) -> usize {
        self.inside.iter().map(Tile::parts).sum::<usize>() + usize::from(self.counted)
    }
}

/// What the data partition is, said in numbers above the map.
pub struct Data {
    pub bytes: u64,
    pub used: Option<u64>,
}

/// Draw the whole page: what the phone is made of in numbers, the map of the
/// room the system takes, the partitions that are not the system's, and what
/// the colours mean.
pub fn page(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, holding: &Tile, others: &[Tile], data: &Data) {
    let dp = |v: f32| t.m.dp(v);
    let margin = t.m.margin;
    let wide = t.m.w as f32 - margin * 2.0;
    let system: u64 = holding.filled() + others.iter().map(Tile::filled).sum::<u64>();

    c.text(&f.title, l.pick(["分区地图", "分區地圖", "Storage map"]), margin, dp(86.0), dp(30.0), t.pal.text, Some(wide));
    let said = format!("{} · {} {}", size(system + data.bytes), holding.parts() + others.len() + 1, l.pick(["个分区", "個分區", "partitions"]));
    c.text(&f.body, &said, margin, dp(112.0), dp(13.0), t.pal.subtitle, Some(wide));

    // Three numbers: the whole, what the files have, and what the system
    // takes. The map below is the third of them.
    let mut x = margin;
    for (value, label) in [
        (size(data.bytes), l.pick(["数据分区", "資料分區", "Data"])),
        (size(system), l.pick(["系统与固件", "系統與韌體", "System"])),
        (
            match data.used {
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

    // The system's own, drawn: the ground it stands on is a card, and each
    // partition a piece of it.
    let label = format!("{} · {}", holding.name, size(holding.filled()));
    c.text(&f.label, &label, margin, dp(222.0), dp(12.0), t.pal.subtitle, Some(wide));
    let map = Rect::new(margin, dp(234.0), wide, dp(440.0));
    c.fill_round_rect(map.x, map.y, map.w, map.h, dp(16.0), t.pal.card);
    let inner = Rect::new(map.x + dp(10.0), map.y + dp(10.0), map.w - dp(20.0), map.h - dp(20.0));
    land(c, t, f, l, inner, &holding.inside);

    // What is not the system's, said rather than drawn: a few tens of
    // megabytes each against a gigabyte of system.
    let top = map.y + map.h + dp(34.0);
    c.text(&f.label, l.pick(["其它分区", "其它分區", "Other partitions"]), margin, top, dp(12.0), t.pal.subtitle, Some(wide));
    let cell = wide / 2.0;
    let row = dp(38.0);
    for (i, tile) in others.iter().enumerate() {
        let (col, line) = (i % 2, i / 2);
        let x = margin + col as f32 * cell;
        let y = top + dp(14.0) + line as f32 * row;
        c.fill_circle(x + dp(4.0), y + dp(9.0), dp(3.5), tile.tone.ink().with_alpha(if t.dark { 235 } else { 210 }));
        c.text(&f.heading, tile.name, x + dp(14.0), y + dp(13.5), dp(13.0), t.pal.text, Some(cell - dp(90.0)));
        let said = size(tile.bytes);
        let room = c.text_width(&f.body, &said, dp(12.5));
        c.text(&f.body, &said, x + cell - dp(12.0) - room, y + dp(13.5), dp(12.5), t.pal.value, None);
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
}

/// Draw `tiles` into `area`, each with what stands on it.
fn land(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, area: Rect, tiles: &[Tile]) {
    let dp = |v: f32| t.m.dp(v);
    for (tile, rect) in tiles.iter().zip(room(area, tiles)) {
        // The room a partition has not given out is a shadow of a tile, not
        // a tile: a package that would fit there is what the map is read for.
        let fill = if tile.counted {
            tile.tone.ink().with_alpha(if t.dark { 54 } else { 40 })
        } else {
            tile.tone.ink().with_alpha(if t.dark { 30 } else { 20 })
        };
        c.fill_round_rect(rect.x, rect.y, rect.w, rect.h, dp(10.0), fill);
        if !tile.inside.is_empty() {
            let said = format!("{} · {}", tile.name, size(tile.filled()));
            c.text(&f.label, &said, rect.x + dp(12.0), rect.y + dp(17.0), dp(11.5), t.pal.subtitle, Some(rect.w - dp(24.0)));
            let inner = Rect::new(rect.x + dp(8.0), rect.y + dp(26.0), rect.w - dp(16.0), rect.h - dp(34.0));
            if inner.h > dp(16.0) && inner.w > dp(16.0) {
                land(c, t, f, l, inner, &tile.inside);
            }
            continue;
        }
        name(c, t, f, l, tile, rect);
    }
}

/// The name of a piece, and its size where there is room for both.
fn name(c: &mut Canvas, t: &Theme, f: &Fonts, l: Lang, tile: &Tile, rect: Rect) {
    let dp = |v: f32| t.m.dp(v);
    let pad = dp(11.0);
    let room = rect.w - pad * 2.0;
    // Narrower than a word, nothing is written: half a name says less than
    // the colour does.
    if room <= dp(44.0) {
        return;
    }
    let said = if tile.counted { tile.name } else { l.pick(["空闲", "空閒", "Free"]) };
    let both = rect.h >= dp(58.0) && room >= dp(64.0);
    // A name is written whole or not at all: a smaller size once, then the
    // colour alone, which says more than `system…` does.
    let mut px = if both { dp(15.0) } else { dp(12.0) };
    if c.text_width(&f.heading, said, px) > room {
        px = dp(10.5);
        if c.text_width(&f.heading, said, px) > room {
            return;
        }
    }
    let base = rect.y + px * 1.62;
    if rect.h < base - rect.y + dp(6.0) {
        return;
    }
    c.text(&f.heading, said, rect.x + pad, base, px, t.pal.text, Some(room));
    if both && rect.h >= base - rect.y + dp(20.0) {
        let size = size(tile.bytes);
        c.text(&f.body, &size, rect.x + pad, base + dp(18.0), dp(12.0), t.pal.subtitle, Some(room));
    }
}

/// How much room a piece takes, as the piece itself would say it: gigabytes
/// to one decimal where it is that large, megabytes below.
fn size(bytes: u64) -> String {
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

/// Where each piece of ground goes: the area cut up in proportion to what
/// each takes, the longer side cut first so that none is a sliver.
fn room(area: Rect, tiles: &[Tile]) -> Vec<Rect> {
    let weights: Vec<f32> = tiles.iter().map(|t| t.filled().max(1) as f32).collect();
    let mut out = vec![Rect::new(0.0, 0.0, 0.0, 0.0); tiles.len()];
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
