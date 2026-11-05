//! Resources for a page: what a caller lets it load, and how much.
//!
//! surimono opens no socket and reads no file. A [`Loader`] is the caller's
//! closure with an allowlist: every URL a page asks for is resolved against
//! the base URL and checked, and only then does the closure see it. Without a
//! loader, or for a URL off the list, the resource just isn't there, as a
//! failed load is in a browser.
//!
//! Nothing a page loads starts a wait on the event loop: scripts are fetched
//! before the first one runs and stylesheets after the last, in tree order, one
//! at a time on the caller's thread. So the order of completions is fixed, and
//! the caps below are what stands between a slow loader and an unbounded
//! render: real time can only reach the output through them.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lol_html::errors::RewritingError;
use lol_html::html_content::{ContentType, Element};
use lol_html::{RewriteStrSettings, element, rewrite_str};
use url::Url;

use crate::charref::decode_attribute;
use crate::error::Error;

/// Most resources one render may load; the rest fail without being asked for.
pub(crate) const MAX_LOADS: usize = 1_000;
/// Real time after the first load, past which the rest fail.
pub(crate) const MAX_LOAD_TIME: Duration = Duration::from_secs(10);
/// Most bytes one render may load.
pub(crate) const MAX_LOAD_BYTES: usize = 256 * 1024 * 1024;

/// The base URL of a page when [`Loader::base_url`] isn't set. Never
/// connected to: it gives a page's relative URLs a form to be checked in.
const DEFAULT_BASE: &str = "http://localhost/";

/// What a page is loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceKind {
    /// An external `<script src>`.
    Script,
    /// A `<link rel="stylesheet" href>`.
    Stylesheet,
}

impl ResourceKind {
    /// A short, stable name, used by the bindings.
    pub fn name(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Stylesheet => "stylesheet",
        }
    }
}

type Fetch<'a> = dyn FnMut(&str, ResourceKind) -> Option<Vec<u8>> + 'a;

/// Your code that gives a page its external scripts and stylesheets, and the
/// URLs it may ask for. Network and files are off until you pass one, and
/// surimono itself never opens either: `fetch` does, if you want it to.
///
/// ```
/// use surimono::{Loader, Options, Renderer, ResourceKind};
///
/// # let font = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/fonts/NotoSans-Regular.ttf"))
/// #     .unwrap_or_else(|e| panic!("{e}"));
/// let renderer = Renderer::new(Options::new().font(font))?;
/// let mut loader = Loader::new(|url, _kind: ResourceKind| {
///     (url == "http://localhost/app.js").then(|| b"document.title = 'loaded'".to_vec())
/// })
/// .allow("/");
/// let html = "<title>x</title><script src=app.js></script>";
/// let page = renderer.render_page_with(html, &mut loader)?;
/// assert!(page.html.contains("<title>loaded</title>"));
/// # Ok::<(), surimono::Error>(())
/// ```
pub struct Loader<'a> {
    fetch: Box<Fetch<'a>>,
    base: String,
    allow: Vec<String>,
}

impl std::fmt::Debug for Loader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loader")
            .field("base", &self.base)
            .field("allow", &self.allow)
            .finish_non_exhaustive()
    }
}

impl<'a> Loader<'a> {
    /// A loader that allows nothing yet. `fetch` gets the absolute URL and
    /// what it is for, and returns the bytes, or `None` if there are none.
    /// Bytes are read as UTF-8.
    ///
    /// It runs on the calling thread, one call at a time, in document order.
    /// surimono can't interrupt it, so give it its own timeout: after 10
    /// seconds of loads, or 1,000 of them, or 256 MiB, the rest fail without
    /// reaching `fetch`, and the page goes on.
    pub fn new(fetch: impl FnMut(&str, ResourceKind) -> Option<Vec<u8>> + 'a) -> Self {
        Self {
            fetch: Box::new(fetch),
            base: DEFAULT_BASE.to_owned(),
            allow: Vec::new(),
        }
    }

    /// Sets the URL a page's relative URLs resolve against, as if the page
    /// had been loaded from it. Default: `http://localhost/`.
    #[must_use]
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base = url.into();
        self
    }

    /// Adds to what a page may load. `url` is absolute or relative to the
    /// base URL, http or https. Ending in `/` it allows everything under that
    /// path (`https://cdn.example.com/lib/`, `/assets/`); otherwise that one
    /// URL (query strings aside). Compared after URLs are parsed, so
    /// `..`, user names and case can't get around it.
    #[must_use]
    pub fn allow(mut self, url: impl Into<String>) -> Self {
        self.allow.push(url.into());
        self
    }
}

/// Replaces each `<link rel="stylesheet">` in `html` with a `<style>` holding
/// what `session` gave for it, or removes it, noting its `href` in `missing`.
/// In tree order, on the page as it stood at the end, so links its scripts
/// added count; a printer has no loader, so this is the only way they print.
pub(crate) fn inline_stylesheets(
    html: &str,
    session: &mut Session<'_, '_>,
    cancel: Option<&AtomicBool>,
    missing: &mut Vec<String>,
) -> Result<String, Error> {
    if !html.contains("<link") {
        return Ok(html.to_owned());
    }
    let session = RefCell::new(session);
    let missing = RefCell::new(missing);
    let on_link = |el: &mut Element<'_, '_>| {
        if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
            return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                "cancelled",
            ));
        }
        let is_sheet = el.get_attribute("rel").is_some_and(|rel| {
            rel.split_ascii_whitespace()
                .any(|t| t.eq_ignore_ascii_case("stylesheet"))
        });
        let Some(href) = el.get_attribute("href").filter(|_| is_sheet) else {
            return Ok(());
        };
        let href = decode_attribute(&href).into_owned();
        match session.borrow_mut().load(&href, ResourceKind::Stylesheet) {
            Some(css) => {
                let media = el
                    .get_attribute("media")
                    .map_or(String::new(), |m| format!(" media=\"{m}\""));
                let css = css.replace("</", "<\\/");
                el.replace(&format!("<style{media}>{css}</style>"), ContentType::Html);
            }
            None => {
                missing.borrow_mut().push(href);
                el.remove();
            }
        }
        Ok(())
    };
    let settings = RewriteStrSettings {
        element_content_handlers: vec![element!("link", on_link)],
        enable_esi_tags: false,
        ..RewriteStrSettings::new()
    };
    rewrite_str(html, settings).map_err(|e| match e {
        RewritingError::ContentHandlerError(_) => Error::Cancelled,
        other => Error::Html(other.to_string()),
    })
}

/// A [`Loader`] checked and running: one per render.
pub(crate) struct Session<'l, 'a> {
    fetch: &'l mut Fetch<'a>,
    base: Url,
    allow: Vec<Url>,
    loads: usize,
    bytes: usize,
    started: Option<Instant>,
}

impl<'l, 'a> Session<'l, 'a> {
    pub(crate) fn new(loader: &'l mut Loader<'a>) -> Result<Self, String> {
        let base = Url::parse(&loader.base)
            .map_err(|e| format!("the loader's base URL {:?}: {e}", loader.base))?;
        let mut allow = Vec::with_capacity(loader.allow.len());
        for entry in &loader.allow {
            let url = base
                .join(entry)
                .map_err(|e| format!("the loader's allow entry {entry:?}: {e}"))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(format!(
                    "the loader's allow entry {entry:?} must be http or https"
                ));
            }
            allow.push(url);
        }
        Ok(Self {
            fetch: &mut *loader.fetch,
            base,
            allow,
            loads: 0,
            bytes: 0,
            started: None,
        })
    }

    /// Whether a URL may be loaded: same origin as an entry, no user name, and
    /// under its path. Origins compare after parsing, so a host can't be
    /// spelled two ways.
    fn allowed(&self, url: &Url) -> bool {
        url.username().is_empty()
            && url.password().is_none()
            && self.allow.iter().any(|entry| {
                entry.origin() == url.origin()
                    && if entry.path().ends_with('/') {
                        url.path().starts_with(entry.path())
                    } else {
                        entry.path() == url.path()
                    }
            })
    }

    /// The text at `raw` as a page wrote it, or `None` if it can't be had.
    pub(crate) fn load(&mut self, raw: &str, kind: ResourceKind) -> Option<String> {
        let url = self.base.join(raw).ok()?;
        if !self.allowed(&url) || self.loads >= MAX_LOADS {
            return None;
        }
        let started = *self.started.get_or_insert_with(Instant::now);
        if started.elapsed() >= MAX_LOAD_TIME {
            return None;
        }
        self.loads += 1;
        let bytes = (self.fetch)(url.as_str(), kind)?;
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > MAX_LOAD_BYTES {
            self.loads = MAX_LOADS;
            return None;
        }
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loads(loader: &mut Loader<'_>, raw: &str, kind: ResourceKind) -> Option<String> {
        Session::new(loader).ok()?.load(raw, kind)
    }

    fn body(url: &str, _: ResourceKind) -> Option<Vec<u8>> {
        Some(url.as_bytes().to_vec())
    }

    #[test]
    fn nothing_is_allowed_until_it_is_listed() {
        let mut l = Loader::new(body);
        assert_eq!(loads(&mut l, "app.js", ResourceKind::Script), None);
        let mut l = Loader::new(body).allow("/assets/");
        assert_eq!(
            loads(&mut l, "/assets/app.js", ResourceKind::Script).as_deref(),
            Some("http://localhost/assets/app.js")
        );
        assert_eq!(loads(&mut l, "app.js", ResourceKind::Script), None);
    }

    #[test]
    fn the_list_cant_be_got_around() {
        let mut l = Loader::new(body)
            .base_url("https://app.example/reports/1")
            .allow("https://cdn.example/lib/")
            .allow("/assets/app.js");
        for (raw, want) in [
            ("https://cdn.example/lib/a.js", true),
            ("https://cdn.example/lib/sub/a.js?v=1#x", true),
            ("https://CDN.example/lib/a.js", true),
            ("/assets/app.js?v=2", true),
            ("https://cdn.example/lib", false),
            ("https://cdn.example/library/a.js", false),
            ("https://cdn.example/lib/../secret.js", false),
            ("https://cdn.example/lib/%2e%2e/secret.js", false),
            ("http://cdn.example/lib/a.js", false),
            ("https://cdn.example:8443/lib/a.js", false),
            ("https://evil.example/lib/a.js", false),
            ("https://cdn.example.evil.example/lib/a.js", false),
            ("https://cdn.example@evil.example/lib/a.js", false),
            ("https://user@cdn.example/lib/a.js", false),
            ("/assets/app.js/more", false),
            ("/assets/other.js", false),
            ("file:///etc/passwd", false),
            ("data:text/javascript,1", false),
            ("javascript:1", false),
            ("http://[", false),
        ] {
            let got = loads(&mut l, raw, ResourceKind::Script).is_some();
            assert_eq!(got, want, "{raw}");
        }
    }

    #[test]
    fn a_bad_list_is_an_error() {
        for mut l in [
            Loader::new(body).base_url("not a url"),
            Loader::new(body).allow("file:///etc/"),
            Loader::new(body).allow("http://["),
        ] {
            assert!(Session::new(&mut l).is_err(), "{l:?}");
        }
    }

    #[test]
    fn caps_stop_the_loads_and_nothing_else() {
        let calls = std::cell::Cell::new(0);
        let mut l = Loader::new(|_, _| {
            calls.set(calls.get() + 1);
            Some(b"x".to_vec())
        })
        .allow("/");
        let mut s = Session::new(&mut l).unwrap_or_else(|e| panic!("{e}"));
        for i in 0..MAX_LOADS {
            assert!(s.load(&format!("/{i}"), ResourceKind::Script).is_some());
        }
        assert_eq!(s.load("/one-more", ResourceKind::Script), None);
        drop(s);
        drop(l);
        assert_eq!(calls.get(), MAX_LOADS);
    }

    #[test]
    fn the_real_time_cap_ends_the_loads() {
        let mut l = Loader::new(body).allow("/");
        let mut s = Session::new(&mut l).unwrap_or_else(|e| panic!("{e}"));
        assert!(s.load("/a", ResourceKind::Script).is_some());
        s.started = Instant::now().checked_sub(MAX_LOAD_TIME);
        assert_eq!(s.load("/b", ResourceKind::Script), None);
    }

    #[test]
    fn the_byte_cap_ends_the_loads() {
        let mut l = Loader::new(|_, _| Some(vec![b'x'; MAX_LOAD_BYTES / 2 + 1])).allow("/");
        let mut s = Session::new(&mut l).unwrap_or_else(|e| panic!("{e}"));
        assert!(s.load("/a", ResourceKind::Script).is_some());
        assert_eq!(s.load("/b", ResourceKind::Script), None);
        assert_eq!(s.load("/c", ResourceKind::Script), None);
    }

    #[test]
    fn bytes_are_read_as_utf8() {
        let mut l = Loader::new(|_, _| Some(b"\xef\xbb\xbfa\xffb".to_vec())).allow("/");
        assert_eq!(
            loads(&mut l, "/a", ResourceKind::Script).as_deref(),
            Some("a\u{fffd}b")
        );
    }

    #[test]
    fn stylesheets_become_style_elements() {
        let mut l = Loader::new(|url, kind| {
            assert_eq!(kind, ResourceKind::Stylesheet);
            url.ends_with("a.css")
                .then(|| b"p{color:red}/*</style><b>*/".to_vec())
        })
        .allow("/");
        let mut s = Session::new(&mut l).unwrap_or_else(|e| panic!("{e}"));
        let mut missing = Vec::new();
        let out = inline_stylesheets(
            r#"<link rel="Stylesheet preload" href="a.css" media="print"><link rel="stylesheet" href="b&#46;css"><link rel="icon" href="c.png"><p>x</p>"#,
            &mut s,
            None,
            &mut missing,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            out,
            r#"<style media="print">p{color:red}/*<\/style><b>*/</style><link rel="icon" href="c.png"><p>x</p>"#
        );
        assert_eq!(missing, ["b.css"]);
    }
}
