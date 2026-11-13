//! A page of the web as the recovery's own browser shows it: the words of
//! its markup, a block at a time, and the links a finger can follow.
//!
//! The recovery carries no layout engine and no script engine, so what is
//! kept is what a page says in its markup: its headings and paragraphs, and
//! the address each link leads to. Scripts, styles and pictures are left out;
//! a page that builds itself with scripts shows nothing, and says so by
//! being empty.

/// A page as it is shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    pub title: String,
    pub blocks: Vec<Block>,
}

/// One block of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Words set as a paragraph.
    Words(String),
    /// Words in a larger hand.
    Heading(String),
    /// Words that lead to another address.
    Link { words: String, to: String },
}

/// How many blocks of one page are kept, and how many words: what a
/// recovery can show without holding a page's whole of it.
const MOST_BLOCKS: usize = 400;
const MOST_WORDS: usize = 200_000;

/// Read `markup` as a page, its links made whole against `base`, the
/// address it was read from.
pub fn parse(markup: &str, base: &str) -> Page {
    let mut built = Built { base, title: String::new(), blocks: Vec::new(), words: String::new(), link: None, heading: false, words_kept: 0 };
    built.run(markup);
    built.flush();
    let mut page = Page { title: built.title.trim().to_string(), blocks: built.blocks };
    if page.title.is_empty() {
        page.title = page
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Heading(words) => Some(words.clone()),
                _ => None,
            })
            .or_else(|| host(base))
            .unwrap_or_default();
    }
    page
}

struct Built<'a> {
    base: &'a str,
    title: String,
    blocks: Vec<Block>,
    /// The words being gathered, in a paragraph or in a link.
    words: String,
    /// The address the words lead to, while they are a link's.
    link: Option<String>,
    heading: bool,
    words_kept: usize,
}

impl Built<'_> {
    fn run(&mut self, markup: &str) {
        let bytes = markup.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'<' {
                let start = i;
                while i < bytes.len() && bytes[i] != b'<' {
                    i += 1;
                }
                self.text(&markup[start..i]);
                continue;
            }
            let Some(end) = markup[i..].find('>').map(|at| i + at) else { break };
            let tag = &markup[i + 1..end];
            i = end + 1;
            let closing = tag.starts_with('/');
            let name: String = tag.trim_start_matches('/').trim_start().chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
            if name.is_empty() {
                continue;
            }
            match name.as_str() {
                "script" | "style" | "noscript" | "template" if !closing => {
                    // Their content is not words: skipped to their end.
                    let end = format!("</{name}");
                    match markup[i..].to_ascii_lowercase().find(end.as_str()) {
                        Some(at) => {
                            i += at + end.len();
                            if let Some(gt) = markup[i..].find('>') {
                                i += gt + 1;
                            }
                        }
                        None => break,
                    }
                }
                "title" if !closing => {
                    let end = markup[i..].to_ascii_lowercase().find("</title").map(|at| i + at);
                    let words = end.map_or(&markup[i..], |at| &markup[i..at]);
                    self.title = collapse(&decode(words));
                    i = end.map_or(bytes.len(), |at| at);
                }
                "a" if !closing => {
                    self.flush();
                    if let Some(href) = attribute(tag, "href").and_then(|href| absolute(self.base, &href)) {
                        self.link = Some(href);
                    }
                }
                "a" => self.flush(),
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if !closing => {
                    self.flush();
                    self.heading = true;
                }
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    self.flush();
                    self.heading = false;
                }
                "p" | "div" | "br" | "li" | "ul" | "ol" | "tr" | "td" | "th" | "table" | "section" | "article" | "header"
                | "footer" | "blockquote" | "pre" | "hr" | "dd" | "dt" => self.flush(),
                _ => {}
            }
        }
    }

    /// Words outside a tag.
    fn text(&mut self, text: &str) {
        let words = decode(text);
        let words = collapse(&words);
        if words.is_empty() {
            return;
        }
        if self.words_kept + words.len() > MOST_WORDS {
            return;
        }
        self.words_kept += words.len();
        if !self.words.is_empty() && !self.words.ends_with(' ') {
            self.words.push(' ');
        }
        self.words.push_str(&words);
    }

    /// The words gathered so far, as the block they make.
    fn flush(&mut self) {
        let words = self.words.trim().to_string();
        self.words.clear();
        if words.is_empty() {
            self.link = None;
            return;
        }
        if self.blocks.len() >= MOST_BLOCKS {
            return;
        }
        match (self.link.take(), self.heading) {
            (Some(to), _) => self.blocks.push(Block::Link { words, to }),
            (None, true) => self.blocks.push(Block::Heading(words)),
            (None, false) => self.blocks.push(Block::Words(words)),
        }
    }
}

/// What a tag holds under `name`, as `href="/a/b"` or `href='/a/b'` holds
/// it.
fn attribute(tag: &str, name: &str) -> Option<String> {
    // The name is looked for in a folded copy, whose offsets are the tag's
    // own: only ASCII is folded, and only ASCII comes before a value.
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let start = from + at;
        let after = start + name.len();
        let before_ok = start == 0 || !lower.as_bytes()[start - 1].is_ascii_alphanumeric();
        if before_ok {
            let rest = &tag[after..];
            let trimmed = rest.trim_start();
            let space = rest.len() - trimmed.len();
            if let Some(rest) = trimmed.strip_prefix('=') {
                let value = rest.trim_start();
                let more = rest.len() - value.len();
                if let Some(quote) = value.chars().next().filter(|c| *c == '"' || *c == '\'') {
                    let at = after + space + 1 + more + 1;
                    let body = &tag[at..];
                    return body.find(quote).map(|end| body[..end].to_string());
                }
            }
        }
        from = after;
    }
    None
}

/// The address `href` leads to, made whole against `base`; none where it
/// leads nowhere a browser here can go.
pub fn absolute(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    let lower = href.to_ascii_lowercase();
    if href.is_empty() || href.starts_with('#') || ["javascript:", "mailto:", "data:", "tel:"].iter().any(|s| lower.starts_with(s)) {
        return None;
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(href.to_string());
    }
    let (scheme, rest) = base.split_once("://")?;
    if let Some(host) = href.strip_prefix("//") {
        return Some(format!("{scheme}://{host}"));
    }
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let origin = format!("{scheme}://{}", &rest[..authority_end]);
    if href.starts_with('/') {
        return Some(format!("{origin}{href}"));
    }
    let path = &rest[authority_end..];
    let dir = match path.rfind('/') {
        Some(at) => &path[..=at],
        None => "/",
    };
    Some(normalize(&format!("{origin}{dir}{href}")))
}

/// A path with its `.` and `..` taken out, its query and fragment kept.
fn normalize(url: &str) -> String {
    let (head, tail) = match url.find(['?', '#']) {
        Some(at) => (&url[..at], &url[at..]),
        None => (url, ""),
    };
    let Some((scheme, rest)) = head.split_once("://") else { return url.to_string() };
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    let mut whole = String::from(scheme);
    whole.push_str("://");
    whole.push_str(authority);
    whole.push('/');
    whole.push_str(&out.join("/"));
    whole.push_str(tail);
    whole
}

/// The host of `url`, for a page that names no title.
fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// The words of `text` with the entities browsers write turned back into
/// characters.
fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        let Some(end) = after.find(';').filter(|&end| end <= 12) else {
            out.push('&');
            rest = &rest[at + 1..];
            continue;
        };
        let name = &after[1..end];
        let turned = match name.to_ascii_lowercase().as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            "times" => Some('×'),
            "copy" => Some('©'),
            "reg" => Some('®'),
            _ => name.strip_prefix('#').and_then(|n| match n.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => n.parse().ok(),
            }).and_then(char::from_u32),
        };
        match turned {
            Some(c) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `text` with its runs of white space made one space each, and none at
/// either end.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_gives_its_words_and_the_addresses_its_links_lead_to() {
        let page = parse(
            "<html><head><title>Skyward &amp; co</title><style>p{color:red}</style></head>\
             <body><h1>发布</h1><p>从<a href=\"/dl/a.zip\">这里</a>下载，或看<a href='https://example.com/b'>那边</a>。</p>\
             <script>var x = '<p>no</p>';</script></body></html>",
            "https://skyward.example/root/page.html",
        );
        assert_eq!(page.title, "Skyward & co");
        assert_eq!(
            page.blocks,
            vec![
                Block::Heading("发布".into()),
                Block::Words("从".into()),
                Block::Link { words: "这里".into(), to: "https://skyward.example/dl/a.zip".into() },
                Block::Words("下载，或看".into()),
                Block::Link { words: "那边".into(), to: "https://example.com/b".into() },
                Block::Words("。".into()),
            ]
        );
    }

    #[test]
    fn a_relative_address_is_made_whole_against_the_page_it_is_on() {
        let base = "https://example.com/a/b/c.html?x=1";
        assert_eq!(absolute(base, "d.zip").as_deref(), Some("https://example.com/a/b/d.zip"));
        assert_eq!(absolute(base, "../e").as_deref(), Some("https://example.com/a/e"));
        assert_eq!(absolute(base, "/f").as_deref(), Some("https://example.com/f"));
        assert_eq!(absolute(base, "//cdn.example/g").as_deref(), Some("https://cdn.example/g"));
        assert_eq!(absolute(base, "http://other/h").as_deref(), Some("http://other/h"));
        assert_eq!(absolute(base, "#top"), None);
        assert_eq!(absolute(base, "javascript:void(0)"), None);
        assert_eq!(absolute(base, ""), None);
    }

    #[test]
    fn a_page_that_says_nothing_useful_is_still_a_page() {
        let page = parse("<script>document.write('hi')</script>", "https://example.com/x");
        assert_eq!(page.blocks, Vec::new());
        assert_eq!(page.title, "example.com");
    }

    #[test]
    fn entities_are_turned_back_into_what_they_stand_for() {
        assert_eq!(decode("a &lt; b &amp;&amp; c &#65; &#x4e2d; &nbsp;x"), "a < b && c A 中  x");
        assert_eq!(decode("100 & 200"), "100 & 200");
        assert_eq!(decode("&unknown;"), "&unknown;");
    }

    #[test]
    fn a_link_that_names_no_href_leads_nowhere_and_is_words() {
        let page = parse("<p><a name=\"x\">锚</a>与<a href=\"#\">顶</a></p>", "https://example.com/");
        assert_eq!(page.blocks, vec![Block::Words("锚与顶".into())]);
    }
}
