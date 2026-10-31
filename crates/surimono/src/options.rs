//! Options for a [`Renderer`](crate::Renderer).

use std::fmt;
use std::time::Duration;

use crate::ErrorPolicy;

/// Resource limits for every script a renderer runs, applied to each island
/// separately.
///
/// A script that goes over a limit stops with [`Error::MemoryLimit`],
/// [`Error::TimeLimit`] or [`Error::StackOverflow`](crate::Error::StackOverflow);
/// it never crashes or hangs the process.
///
/// The limits apply **per island**: every island gets a fresh JavaScript heap of
/// `memory_bytes` and `time` to run, so a document with N islands can take up
/// to N × `time`. Bound the whole document in your own code if you need to.
///
/// Build limits from the defaults, so a limit added in a later release keeps
/// its default in your code:
///
/// ```
/// use std::time::Duration;
/// use surimono::Limits;
///
/// let limits = Limits { time: Duration::from_millis(500), ..Limits::default() };
/// assert_eq!(limits.memory_bytes, Limits::DEFAULT_MEMORY_BYTES);
/// ```
///
/// [`Error::MemoryLimit`]: crate::Error::MemoryLimit
/// [`Error::TimeLimit`]: crate::Error::TimeLimit
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Limits {
    /// Most bytes the JavaScript heap may use. At least 1 MiB.
    pub memory_bytes: usize,
    /// Longest one island's script may run. Must be more than zero.
    pub time: Duration,
    /// Most bytes of native stack the JavaScript engine may use, from 64 KiB to 16 MiB.
    pub stack_bytes: usize,
}

impl Limits {
    /// Default heap limit: 64 MiB.
    pub const DEFAULT_MEMORY_BYTES: usize = 64 * 1024 * 1024;
    /// Default time limit: 2 seconds per island.
    pub const DEFAULT_TIME: Duration = Duration::from_secs(2);
    /// Default stack limit: 512 KiB.
    pub const DEFAULT_STACK_BYTES: usize = 512 * 1024;
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: Self::DEFAULT_MEMORY_BYTES,
            time: Self::DEFAULT_TIME,
            stack_bytes: Self::DEFAULT_STACK_BYTES,
        }
    }
}

/// A JavaScript bundle that renders islands: any script defining a function
/// that takes the island's JSON and returns an SVG string.
#[derive(Clone)]
pub(crate) struct Bundle {
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) entry: String,
}

impl fmt::Debug for Bundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bundle")
            .field("name", &self.name)
            .field("source_bytes", &self.source.len())
            .field("entry", &self.entry)
            .finish()
    }
}

/// How a [`Renderer`](crate::Renderer) is set up: fonts, limits, the random
/// seed, the error policy and which renderers islands may use.
///
/// Every setting has a default, so `Options::new()` is a working start:
///
/// ```
/// use surimono::{ErrorPolicy, Limits, Options};
///
/// let options = Options::new()
///     .seed(7)
///     .limits(Limits::default())
///     .error_policy(ErrorPolicy::Placeholder);
/// # let _ = options;
/// ```
#[derive(Clone, Default)]
pub struct Options {
    pub(crate) fonts: Vec<Vec<u8>>,
    pub(crate) limits: Limits,
    pub(crate) seed: u64,
    pub(crate) error_policy: ErrorPolicy,
    pub(crate) renderers: Option<Vec<String>>,
    pub(crate) bundles: Vec<Bundle>,
    pub(crate) pool_size: Option<usize>,
}

impl Options {
    /// Default options: no fonts, default [`Limits`], seed 0,
    /// [`ErrorPolicy::Fail`], every built-in renderer, a pool sized to the machine.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a font file (TrueType, OpenType or a collection) for measuring text.
    ///
    /// Use the same font files your PDF engine prints with, so labels measured
    /// here fit there. surimono never reads system fonts.
    pub fn font(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.fonts.push(bytes.into());
        self
    }

    /// Sets the resource limits for every script.
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Sets the seed for `Math.random`. The same seed and input give the same output.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Sets what happens when an island fails. Default: [`ErrorPolicy::Fail`].
    pub fn error_policy(mut self, policy: ErrorPolicy) -> Self {
        self.error_policy = policy;
        self
    }

    /// Enables only the named renderers (built-in or added with
    /// [`bundle`](Self::bundle)). By default every renderer is enabled. An island
    /// naming a disabled renderer fails with
    /// [`Error::UnknownRenderer`](crate::Error::UnknownRenderer).
    pub fn renderers<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.renderers = Some(names.into_iter().map(Into::into).collect());
        self
    }

    /// Adds a renderer from your own JavaScript bundle.
    ///
    /// `source` is evaluated once and must define a global function `entry`
    /// that takes the island's spec (parsed JSON) and an object with `width` and
    /// `height`, and returns an SVG string. Islands use it with
    /// `data-renderer="<name>"`. The bundle runs in the same sandbox and limits
    /// as the built-in renderers.
    pub fn bundle(
        mut self,
        name: impl Into<String>,
        source: impl Into<String>,
        entry: impl Into<String>,
    ) -> Self {
        self.bundles.push(Bundle {
            name: name.into(),
            source: source.into(),
            entry: entry.into(),
        });
        self
    }

    /// Sets how many JavaScript runtimes the renderer keeps for concurrent
    /// renders. Default: the number of CPUs, at most 8.
    pub fn pool_size(mut self, size: usize) -> Self {
        self.pool_size = Some(size);
        self
    }
}

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let font_bytes: Vec<usize> = self.fonts.iter().map(Vec::len).collect();
        f.debug_struct("Options")
            .field("font_bytes", &font_bytes)
            .field("limits", &self.limits)
            .field("seed", &self.seed)
            .field("error_policy", &self.error_policy)
            .field("renderers", &self.renderers)
            .field("bundles", &self.bundles)
            .field("pool_size", &self.pool_size)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let o = Options::new();
        assert!(o.fonts.is_empty());
        assert_eq!(o.limits, Limits::default());
        assert_eq!(o.seed, 0);
        assert_eq!(o.error_policy, ErrorPolicy::Fail);
        assert_eq!(o.renderers, None);
        assert!(o.bundles.is_empty());
        assert_eq!(o.pool_size, None);
        assert_eq!(Limits::default().memory_bytes, 64 * 1024 * 1024);
        assert_eq!(Limits::default().time, Duration::from_secs(2));
        assert_eq!(Limits::default().stack_bytes, 512 * 1024);
    }

    #[test]
    fn builder_sets_everything() {
        let o = Options::new()
            .font(vec![1, 2, 3])
            .font(&b"abcd"[..])
            .seed(9)
            .error_policy(ErrorPolicy::Placeholder)
            .renderers(["echarts", "mine"])
            .bundle("mine", "function f(){}", "f")
            .pool_size(3);
        assert_eq!(o.fonts, vec![vec![1, 2, 3], b"abcd".to_vec()]);
        assert_eq!(o.seed, 9);
        assert_eq!(o.error_policy, ErrorPolicy::Placeholder);
        assert_eq!(
            o.renderers,
            Some(vec!["echarts".to_owned(), "mine".to_owned()])
        );
        assert_eq!(o.bundles[0].name, "mine");
        assert_eq!(o.pool_size, Some(3));
    }

    #[test]
    fn debug_does_not_dump_font_or_bundle_bytes() {
        let o = Options::new()
            .font(vec![0u8; 100_000])
            .bundle("b", "x".repeat(50_000), "f");
        let s = format!("{o:?}");
        assert!(s.len() < 500, "{s}");
        assert!(s.contains("100000") && s.contains("50000"), "{s}");
    }
}
