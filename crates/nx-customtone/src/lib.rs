//! Line 6 CustomTone's public listing of HX Stomp presets. CustomTone has no API; this reads the
//! browse pages a browser gets, one page per call. Downloading needs a signed-in account with a
//! registered product and is not here yet.

use scraper::{ElementRef, Html, Selector};

const BROWSE: &str = "https://line6.com/customtone/browse/hx_stomp";
/// Searching has its own pages; the browse pages ignore `search_term`.
const SEARCH: &str = "https://line6.com/customtone/search/hx_stomp";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("CustomTone: {0}")]
    Http(String),
    #[error("CustomTone page not understood: {0}")]
    Page(&'static str),
}

/// The site's sort options, by the value its form sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Name,
    Song,
    Band,
    Guitarist,
    Amp,
    Date,
    Style,
    Downloads,
    Rating,
}

impl Sort {
    pub const ALL: [Sort; 9] =
        [Sort::Rating, Sort::Downloads, Sort::Date, Sort::Name, Sort::Band, Sort::Guitarist, Sort::Song, Sort::Amp, Sort::Style];

    fn param(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Song => "song",
            Self::Band => "band",
            Self::Guitarist => "guitarist",
            Self::Amp => "amp",
            Self::Date => "posted",
            Self::Style => "style",
            Self::Downloads => "thecount",
            Self::Rating => "rating",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Song => "Song",
            Self::Band => "Band",
            Self::Guitarist => "Guitarist",
            Self::Amp => "Amp",
            Self::Date => "Date",
            Self::Style => "Style",
            Self::Downloads => "Downloads",
            Self::Rating => "Rating",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// From 1.
    pub page: u32,
    pub sort: Sort,
    pub descending: bool,
    pub search: String,
}

impl Default for Query {
    fn default() -> Self {
        Self { page: 1, sort: Sort::Rating, descending: true, search: String::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tone {
    pub id: u64,
    pub name: String,
    pub author: String,
    pub band: String,
    pub guitarist: String,
    pub song: String,
    /// The short line under the name; the site calls it style.
    pub style: String,
    pub comment: String,
    /// As the site writes it, m/d/yy.
    pub date: String,
    pub downloads: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowsePage {
    pub tones: Vec<Tone>,
    pub page: u32,
    pub pages: u32,
    pub total: u32,
}

pub fn browse_url(query: &Query) -> String {
    let dir = if query.descending { "desc" } else { "asc" };
    let base = if query.search.trim().is_empty() { BROWSE } else { SEARCH };
    format!("{base}/{}/?sort={}&sort_dir={dir}&search_term={}", query.page.max(1), query.sort.param(), encode(&query.search))
}

/// One page of the listing, fetched now.
pub fn fetch(query: &Query) -> Result<BrowsePage, Error> {
    let mut response = ureq::get(&browse_url(query))
        .header("User-Agent", concat!("nx-edit/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| Error::Http(e.to_string()))?;
    let html = response.body_mut().read_to_string().map_err(|e| Error::Http(e.to_string()))?;
    parse_browse(&html)
}

pub fn parse_browse(html: &str) -> Result<BrowsePage, Error> {
    let doc = Html::parse_document(html);
    let pagination = first_text(doc.root_element(), "#ct_pagination").ok_or(Error::Page("no pagination"))?;
    let (page, pages) = page_of(&pagination).ok_or(Error::Page("no \"Page n of m\""))?;
    let total = first_text(doc.root_element(), "#tone-count")
        .and_then(|t| leading_number(&t))
        .ok_or(Error::Page("no tone count"))? as u32;
    let tones = doc.select(&selector("div.tone")).map(tone).collect::<Result<_, _>>()?;
    Ok(BrowsePage { tones, page, pages, total })
}

fn tone(el: ElementRef) -> Result<Tone, Error> {
    let id = el
        .select(&selector("a[href^='/customtone/tone/']"))
        .find_map(|a| a.value().attr("href")?.trim_start_matches("/customtone/tone/").trim_end_matches('/').parse().ok())
        .ok_or(Error::Page("tone without an id"))?;
    let mut tone = Tone { id, ..Tone::default() };
    tone.author = first_text(el, "a[href^='/customtone/profile/']").unwrap_or_default();
    tone.comment = first_text(el, "div.comment").unwrap_or_default();
    tone.date = first_text(el, ".date").unwrap_or_default();
    for li in el.select(&selector("div.details ul li")) {
        let line = squash(&li.text().collect::<String>());
        let field = |label: &str| line.strip_prefix(label).map(|v| v.trim().to_owned());
        if let Some(v) = field("TONE NAME:") {
            tone.name = v;
        } else if let Some(v) = field("BAND:") {
            tone.band = v;
        } else if let Some(v) = field("GUITARIST:") {
            tone.guitarist = v;
        } else if let Some(v) = field("SONG:") {
            tone.song = v;
        } else if tone.style.is_empty() && !line.is_empty() {
            tone.style = line;
        }
    }
    let text = squash(&el.text().collect::<String>());
    tone.downloads = text
        .split(" downloads")
        .next()
        .filter(|_| text.contains(" downloads"))
        .and_then(|before| before.rsplit(' ').next()?.parse().ok())
        .unwrap_or_default();
    Ok(tone)
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("selectors here are valid")
}

fn first_text(el: ElementRef, css: &str) -> Option<String> {
    el.select(&selector(css)).next().map(|e| squash(&e.text().collect::<String>()))
}

/// Runs of whitespace, newlines included, become one space.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn leading_number(text: &str) -> Option<u64> {
    text.split_whitespace().next()?.parse().ok()
}

fn page_of(text: &str) -> Option<(u32, u32)> {
    let rest = text.split("Page ").nth(1)?;
    let mut words = rest.split_whitespace();
    let page = words.next()?.parse().ok()?;
    (words.next()? == "of").then_some(())?;
    Some((page, words.next()?.parse().ok()?))
}

/// Percent-encodes everything but RFC 3986's unreserved characters.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
