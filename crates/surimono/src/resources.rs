//! What a page's `<script src>` may load: nothing, unless the caller says so.

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// Loads what a page's external scripts name, for a caller that wants them run.
///
/// Without one, a render opens no network connection and reads no file: every
/// `<script src>` fails with [`ResourceFailureReason::NoLoader`] in the report.
/// surimono itself never does any I/O for a page, with or without a loader:
/// the only code that does is the function you pass in.
///
/// The URL is the `src` attribute as written, with no base applied. It reaches
/// the function only if the allowlist allows it. An allowlist entry allows a
/// URL equal to it; an entry ending in `/` also allows URLs under it, unless
/// the rest of the path has a `.` or `..` segment, a backslash, a control
/// character or an escaped dot, slash or backslash, which could climb out.
///
/// Scripts load one at a time, in document order, on the render's worker
/// thread and before any script runs, so what they do can't depend on which
/// finished first. The function runs once per URL in a render. One render
/// makes at most 1,000 loads, takes at most 10 seconds of real time from the
/// first, and holds at most [`Limits::memory_bytes`](crate::Limits::memory_bytes)
/// of script source; past a cap the rest fail in the report and the page goes
/// on without them. The time cap is looked at between loads, so the function
/// must bound its own wait for a single resource.
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
        self.allow.iter().any(|entry| {
            url == entry
                || (entry.ends_with('/')
                    && url
                        .strip_prefix(entry.as_str())
                        .is_some_and(stays_under_the_prefix))
        })
    }
}

impl fmt::Debug for ResourceLoader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceLoader")
            .field("allow", &self.allow)
            .finish_non_exhaustive()
    }
}

/// Whether `tail`, what follows an allowlist prefix, can't leave the prefix.
fn stays_under_the_prefix(tail: &str) -> bool {
    let path = tail.split(['?', '#']).next().unwrap_or_default();
    let lower = path.to_ascii_lowercase();
    !path.chars().any(|c| c == '\\' || c.is_control())
        && !["%2e", "%2f", "%5c"].iter().any(|e| lower.contains(e))
        && !path.split('/').any(|segment| matches!(segment, "." | ".."))
}

/// What a page tried to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceKind {
    /// A `<script src>`.
    Script,
    /// An `<img src>`.
    Image,
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
    /// The script's bytes aren't UTF-8.
    NotUtf8,
    /// The render had made 1,000 loads.
    TooManyLoads,
    /// The render's loads had taken 10 seconds of real time.
    OutOfTime,
    /// The render's scripts would pass
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
            Self::TooLarge => f.write_str("the render's scripts would pass the memory limit"),
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
    loader: Option<&'a ResourceLoader>,
    /// Script source bytes this render may still hold.
    room: usize,
    loads: usize,
    /// When the first load started. The only clock in a render, and only
    /// with a loader.
    started: Option<Instant>,
    /// Each URL's result, so a URL repeated in the page loads once.
    loaded: HashMap<String, Result<String, ResourceFailureReason>>,
    pub(crate) failures: Failures,
}

impl<'a> Fetcher<'a> {
    pub(crate) fn new(loader: Option<&'a ResourceLoader>, room: usize) -> Self {
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
        let result = match self.loaded.get(url) {
            Some(known) => known.clone(),
            None => {
                let fresh = self.load(url);
                self.loaded.insert(url.to_owned(), fresh.clone());
                fresh
            }
        };
        let result = result.and_then(|source| {
            // Every use of a repeated URL is another copy held until it runs.
            self.room = self
                .room
                .checked_sub(source.len())
                .ok_or(ResourceFailureReason::TooLarge)?;
            Ok(source)
        });
        match result {
            Ok(source) => Some(source),
            Err(reason) => {
                self.failures.add(ResourceKind::Script, url, reason);
                None
            }
        }
    }

    fn load(&mut self, url: &str) -> Result<String, ResourceFailureReason> {
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
        let bytes = catch_unwind(AssertUnwindSafe(|| (loader.load)(url)))
            .unwrap_or_else(|_| Err("it panicked".to_owned()))
            .map_err(|message| ResourceFailureReason::Failed(capped(&message)))?;
        if bytes.len() > self.room {
            return Err(ResourceFailureReason::TooLarge);
        }
        let source = String::from_utf8(bytes).map_err(|_| ResourceFailureReason::NotUtf8)?;
        Ok(match source.strip_prefix('\u{feff}') {
            Some(rest) => rest.to_owned(),
            None => source,
        })
    }
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
            "https://cdn.example/lib/a.js?next=../../b",
            "https://cdn.example/lib/a..b.js",
        ] {
            assert!(l.allows(ok), "{ok}");
        }
        for no in [
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
        let mut f = Fetcher::new(Some(&l), 1 << 20);
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
        let mut f = Fetcher::new(Some(&l), 150);
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
        let mut f = Fetcher::new(Some(&l), 1 << 20);
        for i in 0..MAX_LOADS + 5 {
            let _ = f.script(&format!("x/{i}"));
        }
        assert_eq!(calls.load(Ordering::Relaxed), MAX_LOADS);
        assert_eq!(f.failures.count, 5);
        assert_eq!(
            f.failures.kept[0].reason,
            ResourceFailureReason::TooManyLoads
        );

        let mut f = Fetcher::new(Some(&l), 1 << 20);
        assert!(f.script("x/a").is_some());
        f.started = Instant::now().checked_sub(MAX_LOAD_TIME);
        assert!(f.started.is_some(), "the clock is older than the cap");
        assert_eq!(f.script("x/b"), None);
        assert_eq!(f.failures.kept[0].reason, ResourceFailureReason::OutOfTime);
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
        let mut f = Fetcher::new(Some(&l), 1 << 20);
        assert_eq!(f.script("a.js").as_deref(), Some("1"));
        assert_eq!(f.script("b.js"), None);
        assert_eq!(f.failures.kept[0].reason, ResourceFailureReason::NotAllowed);
        assert!(format!("{l:?}").contains("a.js"));
    }
}
