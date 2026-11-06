//! What a page's `<script src>`, `<iframe src>` and `<link rel="stylesheet">`
//! may load:
//! nothing, unless the caller says so.

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::{Duration, Instant};

use html5ever::{LocalName, QualName};

use crate::error::Error;
use crate::print::{Gate, Tree};

/// Most loads one render may start; the rest fail without calling the loader.
pub(crate) const MAX_LOADS: usize = 1_000;
/// Most real time one render's loads may take, from the first. The rest fail
/// without calling the loader. Only a loader can make a render see real time.
pub(crate) const MAX_LOAD_TIME: Duration = Duration::from_secs(10);
/// Most entries in [`PageReport::failed`](crate::PageReport::failed); the rest
/// are only counted.
const MAX_KEPT_FAILURES: usize = 64;
/// Longest URL or message kept in a report.
const MAX_KEPT_BYTES: usize = 2 * 1024;

/// Loads what a page's external scripts, frames and stylesheets name, for a
/// caller that wants them.
///
/// Without one, a render opens no network connection and reads no file: every
/// `<script src>` and `<iframe src>` fails with
/// [`ResourceFailureReason::NoLoader`] in the report, and every `<link>` is
/// left for the printer.
/// surimono itself never does any I/O for a page, with or without a loader:
/// the only code that does is the function you pass in.
///
/// The URL is the attribute as written, with no base applied. It reaches the
/// function only if the allowlist allows it. An allowlist entry allows a URL
/// equal to it; an entry ending in `/` also allows URLs under it, unless what
/// follows the entry has a segment of only dots and spaces (Win32 strips those
/// from a segment's end, so `.. ` is `..` there), an empty segment, a `;`, a
/// backslash, a control character, a `:` in its first segment, or an escape
/// other than of an ordinary character, query included, any of which could
/// climb out of a directory the entry stands for. Only ASCII is looked at: a
/// function that normalizes Unicode, where `．．` (full-width) and `‥` become
/// `..`, must do so before it checks. A function that reads files should
/// always check what it opens.
///
/// A stylesheet comes out as a `<style>` holding what the function gave, so a
/// printer needs no access to it; a `url()` or `@import` in it isn't followed.
/// An allowed stylesheet that can't be had goes from the page, and into the
/// report. A `<link>` off the list is left for the printer, as all are without
/// a loader: what a printer may read is for its own configuration.
///
/// Scripts load one at a time, in document order, on the render's worker
/// thread and before any script runs, so what they do can't depend on which
/// finished first. An `<iframe src>` loads the same way, under the same
/// allowlist, with its URL's `#fragment` taken off, and fails in the report as a
/// [`ResourceKind::Script`]. Unlike a script, a frame can be added by one, so
/// loads can happen while a page runs, under its time limit, and it can read
/// back what it asked for: don't let through what a page mustn't see.
/// Stylesheets load the same way once the scripts have settled, so a `<link>` a
/// script added counts. The function runs once per URL in a
/// render. One render makes at most 1,000 loads, takes at most 10 seconds of
/// real time from the first, and holds at most
/// [`Limits::memory_bytes`](crate::Limits::memory_bytes) of source, each use of
/// a repeated URL counted; past a cap the rest fail in the report and the page
/// goes on without them. The time cap is looked at between loads, so the
/// function must bound its own wait for a single resource. Stylesheets also
/// fall under the time limit for everything after the scripts: past it the
/// render fails with [`Error::PageTimeLimit`](crate::Error::PageTimeLimit).
///
/// ```
/// use surimono::{Options, Renderer, ResourceLoader};
///
/// # let font = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/fonts/NotoSans-Regular.ttf"))
/// #     .unwrap_or_else(|e| panic!("{e}"));
/// let loader = ResourceLoader::new(["assets/"], |url| match url {
///     "assets/title.js" => Ok(b"document.title = 'Q3';".to_vec()),
///     _ => Err("no such file".to_owned()),
/// });
/// let renderer = Renderer::new(Options::new().font(font).resource_loader(loader))?;
/// let page = renderer.render_page("<!doctype html><title></title><script src='assets/title.js'></script>")?;
/// assert!(page.html.contains("<title>Q3</title>"));
/// # Ok::<(), surimono::Error>(())
/// ```
#[derive(Clone)]
pub struct ResourceLoader {
    allow: Arc<[String]>,
    load: Arc<LoadFn>,
}

type LoadFn = dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync;

impl ResourceLoader {
    /// A loader calling `load` for URLs `allow` allows. `load` returns the
    /// file's bytes, or why it can't; the message goes in the report. It must
    /// return the same bytes for the same URL, or the same input stops giving
    /// the same output.
    pub fn new<I, S, F>(allow: I, load: F) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
        F: Fn(&str) -> Result<Vec<u8>, String> + Send + Sync + 'static,
    {
        Self {
            allow: allow.into_iter().map(Into::into).collect(),
            load: Arc::new(load),
        }
    }

    /// A loader that serves these bytes and nothing else: the URLs are the
    /// allowlist, matched exactly, and no I/O happens during a render.
    pub fn from_map<I, S>(files: I) -> Self
    where
        I: IntoIterator<Item = (S, Vec<u8>)>,
        S: Into<String>,
    {
        let files: HashMap<String, Vec<u8>> =
            files.into_iter().map(|(u, b)| (u.into(), b)).collect();
        let mut allow: Vec<String> = files.keys().cloned().collect();
        // A trailing `/` would make an entry a prefix, but the map only has
        // exact URLs, and sorted the list doesn't depend on hash order.
        allow.sort_unstable();
        Self::new(allow, move |url| {
            files
                .get(url)
                .cloned()
                .ok_or_else(|| "not in the loader's map".to_owned())
        })
    }

    /// Whether the allowlist lets `url` through.
    fn allows(&self, url: &str) -> bool {
        allows(&self.allow, url)
    }
}

/// Where a render's resources come from.
#[derive(Clone, Copy)]
pub(crate) enum Source<'a> {
    /// A loader from the options, called on the worker.
    Shared(&'a ResourceLoader),
    /// A function that has to run on the caller's thread, behind `allow`.
    Caller {
        allow: &'a [String],
        ask: &'a dyn Fn(&str) -> Result<Vec<u8>, String>,
    },
}

impl Source<'_> {
    fn allows(&self, url: &str) -> bool {
        match self {
            Self::Shared(loader) => loader.allows(url),
            Self::Caller { allow, .. } => allows(allow, url),
        }
    }

    fn load(&self, url: &str) -> Result<Vec<u8>, String> {
        surimono_core::paused(|| match self {
            Self::Shared(loader) => (loader.load)(url),
            Self::Caller { ask, .. } => ask(url),
        })
    }
}

/// Whether `allow` lets `url` through; see [`ResourceLoader`].
fn allows(allow: &[String], url: &str) -> bool {
    allow.iter().any(|entry| {
        url == entry
            || (entry.ends_with('/')
                && url
                    .strip_prefix(entry.as_str())
                    .is_some_and(stays_under_the_prefix))
    })
}

impl fmt::Debug for ResourceLoader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceLoader")
            .field("allow", &self.allow)
            .finish_non_exhaustive()
    }
}

/// Whether `tail`, what follows an allowlist prefix, can't leave the prefix.
/// The whole tail, query and fragment too: they mean nothing to a function
/// that treats the URL as a path.
fn stays_under_the_prefix(tail: &str) -> bool {
    if tail
        .chars()
        .any(|c| c == '\\' || c == ';' || c.is_control())
    {
        return false;
    }
    let first = tail.split('/').next().unwrap_or_default();
    // A drive (`C:/windows`) replaces the root when a path is joined.
    if first.contains(':') {
        return false;
    }
    // A final empty segment is a directory; any other is an absolute path.
    let mut segments: Vec<&str> = tail.split('/').collect();
    if segments.last().is_some_and(|s| s.is_empty()) {
        segments.pop();
    }
    if segments.iter().any(|s| only_dots_and_spaces(s)) {
        return false;
    }
    !has_risky_escape(tail)
}

/// Whether a segment is nothing, or has only dots and spaces, an escaped space
/// included.
fn only_dots_and_spaces(segment: &str) -> bool {
    segment
        .replace("%20", " ")
        .chars()
        .all(|c| c == '.' || c == ' ')
}

/// Whether `text` has a `%` that isn't an escape of a letter, digit or one of
/// a few ordinary punctuation marks. So no escaped dot, slash, backslash or
/// control character, and no `%25`, which would hide any of them one level
/// down.
fn has_risky_escape(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.iter().enumerate().any(|(i, b)| {
        *b == b'%'
            && !bytes
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())
                .is_some_and(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, b' ' | b'-' | b'_' | b'+')
                })
    })
}

/// What a page tried to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceKind {
    /// A `<script src>`, or an `<iframe src>`.
    Script,
    /// An `<img src>`.
    Image,
    /// A `<link rel="stylesheet" href>`.
    Stylesheet,
}

/// Why a resource didn't reach the page.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceFailureReason {
    /// No [`ResourceLoader`] was given: network and files are off.
    NoLoader,
    /// The loader's allowlist doesn't allow this URL.
    NotAllowed,
    /// The loader failed, with its message (cut to a few KiB).
    Failed(String),
    /// The file's bytes aren't UTF-8.
    NotUtf8,
    /// The render had made 1,000 loads.
    TooManyLoads,
    /// The render's loads had taken 10 seconds of real time.
    OutOfTime,
    /// The render's scripts and stylesheets would pass
    /// [`Limits::memory_bytes`](crate::Limits::memory_bytes).
    TooLarge,
    /// surimono doesn't load images: a page's `load` and `error` events never
    /// fire for one, and `src` stays for the PDF engine to load.
    NotLoaded,
}

impl fmt::Display for ResourceFailureReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLoader => f.write_str("no resource loader was given"),
            Self::NotAllowed => f.write_str("not in the loader's allowlist"),
            Self::Failed(message) => write!(f, "the loader failed: {message}"),
            Self::NotUtf8 => f.write_str("it isn't UTF-8"),
            Self::TooManyLoads => f.write_str("the render's cap of 1000 loads was reached"),
            Self::OutOfTime => f.write_str("the render's 10 seconds of loading were used up"),
            Self::TooLarge => f.write_str("the render's resources would pass the memory limit"),
            Self::NotLoaded => f.write_str("images aren't loaded during a render"),
        }
    }
}

/// A resource a page named that it didn't get.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceFailure {
    /// What it is.
    pub kind: ResourceKind,
    /// The URL as written in the document, cut to a few KiB.
    pub url: String,
    /// Why it failed.
    pub reason: ResourceFailureReason,
}

/// The failures of one render: the first few, and how many there were.
#[derive(Debug, Default)]
pub(crate) struct Failures {
    pub(crate) kept: Vec<ResourceFailure>,
    pub(crate) count: u64,
}

impl Failures {
    pub(crate) fn add(&mut self, kind: ResourceKind, url: &str, reason: ResourceFailureReason) {
        self.count += 1;
        if self.kept.len() < MAX_KEPT_FAILURES {
            self.kept.push(ResourceFailure {
                kind,
                url: capped(url),
                reason,
            });
        }
    }
}

/// `text` cut to [`MAX_KEPT_BYTES`] on a character boundary.
fn capped(text: &str) -> String {
    let mut cut = text.len().min(MAX_KEPT_BYTES);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text[..cut].to_owned()
}

/// One render's loads, holding its caps.
pub(crate) struct Fetcher<'a> {
    loader: Option<Source<'a>>,
    /// Script source bytes this render may still hold.
    room: usize,
    loads: usize,
    /// When the first load started. With a loader, this is a clock in a
    /// render that only the loader's speed can move.
    started: Option<Instant>,
    /// Each URL's result, so a URL repeated in the page loads once and a
    /// repeat costs nothing until it fits.
    loaded: HashMap<String, Result<Arc<str>, ResourceFailureReason>>,
    pub(crate) failures: Failures,
}

impl<'a> Fetcher<'a> {
    pub(crate) fn new(loader: Option<Source<'a>>, room: usize) -> Self {
        Self {
            loader,
            room,
            loads: 0,
            started: None,
            loaded: HashMap::new(),
            failures: Failures::default(),
        }
    }

    /// The source of the script at `url`, or `None` with a failure recorded.
    pub(crate) fn script(&mut self, url: &str) -> Option<String> {
        self.text(ResourceKind::Script, url)
    }

    /// Whether a loader was given, so a page's `<link>`s may be ours to resolve.
    pub(crate) fn has_loader(&self) -> bool {
        self.loader.is_some()
    }

    pub(crate) fn allows(&self, url: &str) -> bool {
        self.loader.is_some_and(|l| l.allows(url))
    }

    /// The text at `url`, or `None` with a failure recorded under `kind`.
    pub(crate) fn text(&mut self, kind: ResourceKind, url: &str) -> Option<String> {
        let result = match self.loaded.get(url) {
            Some(known) => known.clone(),
            None => {
                let fresh = self.load(url);
                self.loaded.insert(url.to_owned(), fresh.clone());
                fresh
            }
        };
        let result = result.and_then(|source| {
            // Every use of a repeated URL is another copy held until it runs,
            // made only once it fits.
            self.room = self
                .room
                .checked_sub(source.len())
                .ok_or(ResourceFailureReason::TooLarge)?;
            Ok(source.as_ref().to_owned())
        });
        match result {
            Ok(source) => Some(source),
            Err(reason) => {
                self.failures.add(kind, url, reason);
                None
            }
        }
    }

    fn load(&mut self, url: &str) -> Result<Arc<str>, ResourceFailureReason> {
        let loader = self.loader.ok_or(ResourceFailureReason::NoLoader)?;
        if !loader.allows(url) {
            return Err(ResourceFailureReason::NotAllowed);
        }
        if self.loads == MAX_LOADS {
            return Err(ResourceFailureReason::TooManyLoads);
        }
        if self.started.is_some_and(|s| s.elapsed() >= MAX_LOAD_TIME) {
            return Err(ResourceFailureReason::OutOfTime);
        }
        self.started.get_or_insert_with(Instant::now);
        self.loads += 1;
        let bytes = catch_unwind(AssertUnwindSafe(|| loader.load(url)))
            .unwrap_or_else(|_| Err("it panicked".to_owned()))
            .map_err(|message| ResourceFailureReason::Failed(capped(&message)))?;
        if bytes.len() > self.room {
            return Err(ResourceFailureReason::TooLarge);
        }
        let source = String::from_utf8(bytes).map_err(|_| ResourceFailureReason::NotUtf8)?;
        Ok(Arc::from(
            source.strip_prefix('\u{feff}').unwrap_or(&source),
        ))
    }
}

/// Replaces each `<link rel="stylesheet">` in `tree`, the page as a printer
/// reads it, with a `<style>` holding what `fetcher` gave for it, or removes
/// it, in tree order. Links a page's scripts added count. Links off the
/// allowlist stay.
pub(crate) fn inline_stylesheets(
    tree: &mut Tree,
    fetcher: &mut Fetcher<'_>,
    gate: &Gate<'_>,
) -> Result<(), Error> {
    for id in tree.elements_named("link") {
        gate.check()?;
        let Some(link) = tree.element(id) else {
            continue;
        };
        let is_sheet = link.attr("rel").is_some_and(|rel| {
            rel.split_ascii_whitespace()
                .any(|t| t.eq_ignore_ascii_case("stylesheet"))
        });
        let Some(href) = link.attr("href").filter(|_| is_sheet) else {
            continue;
        };
        if !fetcher.allows(href) {
            continue;
        }
        let href = href.to_owned();
        let Some(css) = fetcher.text(ResourceKind::Stylesheet, &href) else {
            tree.detach(id);
            continue;
        };
        if let Some(element) = tree.element_mut(id) {
            element.name = QualName::new(None, element.name.ns.clone(), LocalName::from("style"));
            element.attrs.retain(|a| a.name.local.as_ref() == "media");
        }
        // Text can't end the element it goes into; `\/` is `/` in CSS.
        tree.set_text(id, css.replace("</", "<\\/"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn serve() -> ResourceLoader {
        ResourceLoader::new(["https://cdn.example/lib/", "exact.js"], |url| {
            Ok(url.as_bytes().to_vec())
        })
    }

    #[test]
    fn the_allowlist_matches_exactly_or_under_a_slash() {
        let l = serve();
        for ok in [
            "exact.js",
            "https://cdn.example/lib/a.js",
            "https://cdn.example/lib/x/y.js?v=1#z",
            "https://cdn.example/lib/a..b.js",
            "https://cdn.example/lib/a%20b.js",
            "https://cdn.example/lib/dir/",
        ] {
            assert!(l.allows(ok), "{ok}");
        }
        let assets = ResourceLoader::new(["assets/"], |_| Ok(Vec::new()));
        for no in [
            "assets//etc/passwd",
            "assets/%252e%252e/secret",
            "assets/a/..;/..;/secret",
            "assets/%00/x",
            "assets/%0a/x",
            "assets/x?/../../secret",
            "assets/x#/../../secret",
            "assets/C:/windows/x",
            "assets/.. /secret",
            "assets/..%20/secret",
            "assets/.../secret",
            "assets/ /secret",
            "assets/%20/secret",
            "assets/x/.. ",
        ] {
            assert!(!assets.allows(no), "{no}");
        }
        for no in [
            "https://cdn.example/lib//evil.test/a.js",
            "https://cdn.example/lib/a.js?next=../../b",
            "exact.js/",
            "exact.js?v=2",
            "https://cdn.example/lib",
            "https://cdn.example/libx/a.js",
            "https://cdn.example.evil.test/lib/a.js",
            "https://cdn.example/lib/../secret.js",
            "https://cdn.example/lib/a/../../secret.js",
            "https://cdn.example/lib/./a.js",
            "https://cdn.example/lib/%2e%2e/secret.js",
            "https://cdn.example/lib/%2E./secret.js",
            "https://cdn.example/lib/..%2Fsecret.js",
            "https://cdn.example/lib/..%5csecret.js",
            "https://cdn.example/lib/..\\secret.js",
            "https://cdn.example/lib/a\n.js",
            "/etc/passwd",
            "",
        ] {
            assert!(!l.allows(no), "{no:?}");
        }
    }

    #[test]
    fn without_a_loader_nothing_loads() {
        let mut f = Fetcher::new(None, 1 << 20);
        assert_eq!(f.script("a.js"), None);
        assert_eq!(f.failures.count, 1);
        assert_eq!(f.failures.kept[0].kind, ResourceKind::Script);
        assert_eq!(f.failures.kept[0].reason, ResourceFailureReason::NoLoader);
    }

    #[test]
    fn a_refused_url_never_reaches_the_function() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let l = ResourceLoader::new(["ok/"], move |_| {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(Vec::new())
        });
        let mut f = Fetcher::new(Some(Source::Shared(&l)), 1 << 20);
        assert_eq!(f.script("no.js"), None);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(f.failures.kept[0].reason, ResourceFailureReason::NotAllowed);
        assert_eq!(f.script("ok/a.js").as_deref(), Some(""));
        assert_eq!(f.script("ok/a.js").as_deref(), Some(""));
        assert_eq!(calls.load(Ordering::Relaxed), 1, "once per URL");
    }

    #[test]
    fn failures_and_bad_bytes_are_typed() {
        let l = ResourceLoader::new(["x/"], |url| match url {
            "x/fail" => Err("gone".to_owned()),
            "x/panic" => panic!("loader bug"),
            "x/latin1" => Ok(vec![0xE9]),
            "x/bom" => Ok(b"\xEF\xBB\xBFa".to_vec()),
            _ => Ok(vec![b'y'; 100]),
        });
        let mut f = Fetcher::new(Some(Source::Shared(&l)), 150);
        assert_eq!(f.script("x/bom").as_deref(), Some("a"));
        for url in ["x/fail", "x/panic", "x/latin1"] {
            assert_eq!(f.script(url), None, "{url}");
        }
        assert_eq!(
            f.failures
                .kept
                .iter()
                .map(|e| e.reason.clone())
                .collect::<Vec<_>>(),
            [
                ResourceFailureReason::Failed("gone".into()),
                ResourceFailureReason::Failed("it panicked".into()),
                ResourceFailureReason::NotUtf8,
            ]
        );
        // The second copy of a repeated URL doesn't fit the budget.
        assert!(f.script("x/big").is_some());
        assert_eq!(f.script("x/big"), None);
        assert_eq!(f.failures.count, 4);
        assert_eq!(f.failures.kept[3].reason, ResourceFailureReason::TooLarge);
    }

    #[test]
    fn caps_stop_loading_and_the_page_goes_on() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let l = ResourceLoader::new(["x/"], move |_| {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(Vec::new())
        });
        let mut f = Fetcher::new(Some(Source::Shared(&l)), 1 << 20);
        for i in 0..MAX_LOADS + 5 {
            let _ = f.script(&format!("x/{i}"));
        }
        assert_eq!(calls.load(Ordering::Relaxed), MAX_LOADS);
        assert_eq!(f.failures.count, 5);
        assert_eq!(
            f.failures.kept[0].reason,
            ResourceFailureReason::TooManyLoads
        );

        let mut f = Fetcher::new(Some(Source::Shared(&l)), 1 << 20);
        assert!(f.script("x/a").is_some());
        f.started = Instant::now().checked_sub(MAX_LOAD_TIME);
        assert!(f.started.is_some(), "the clock is older than the cap");
        assert_eq!(f.script("x/b"), None);
        assert_eq!(f.failures.kept[0].reason, ResourceFailureReason::OutOfTime);
    }

    #[test]
    fn a_repeat_that_cant_fit_costs_nothing() {
        // 50,000 uses of an 8 MiB file: copied before the check, 400 GB.
        let l = ResourceLoader::new(["x/"], |_| Ok(vec![b'a'; 8 << 20]));
        let mut f = Fetcher::new(Some(Source::Shared(&l)), 9 << 20);
        assert!(f.text(ResourceKind::Stylesheet, "x/a").is_some());
        let started = Instant::now();
        for _ in 0..50_000 {
            assert_eq!(f.text(ResourceKind::Stylesheet, "x/a"), None);
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(f.failures.count, 50_000);
        assert_eq!(f.failures.kept[0].reason, ResourceFailureReason::TooLarge);
    }

    #[test]
    fn reports_stay_small() {
        let mut f = Fetcher::new(None, 0);
        let long = "é".repeat(10_000);
        for _ in 0..1000 {
            let _ = f.script(&long);
        }
        assert_eq!(f.failures.count, 1000);
        assert_eq!(f.failures.kept.len(), MAX_KEPT_FAILURES);
        assert!(f.failures.kept[0].url.len() <= MAX_KEPT_BYTES);
    }

    #[test]
    fn from_map_serves_its_urls_and_no_others() {
        let l = ResourceLoader::from_map([("a.js", b"1".to_vec())]);
        let mut f = Fetcher::new(Some(Source::Shared(&l)), 1 << 20);
        assert_eq!(f.script("a.js").as_deref(), Some("1"));
        assert_eq!(f.script("b.js"), None);
        assert_eq!(f.failures.kept[0].reason, ResourceFailureReason::NotAllowed);
        assert!(format!("{l:?}").contains("a.js"));
    }
}
