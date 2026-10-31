//! Built-in renderers, each behind a Cargo feature.
//!
//! Each one is a committed, minified JavaScript bundle under `bundles/`,
//! built by `js/build.sh` from pinned npm packages, with its licence files
//! next to it. A runner compiles a built-in the first time a render asks for
//! it, so unused renderers cost nothing.

/// A renderer compiled into this build.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Builtin {
    /// The renderer name islands ask for.
    pub(crate) name: &'static str,
    /// The bundle's JavaScript source.
    pub(crate) source: &'static str,
    /// The global function the bundle defines.
    pub(crate) entry: &'static str,
}

#[cfg(feature = "echarts")]
const ECHARTS: Builtin = Builtin {
    name: "echarts",
    source: include_str!("../bundles/echarts/echarts.min.js"),
    entry: "entry",
};

/// Vega and Vega-Lite share one bundle with an entry for each.
#[cfg(feature = "vega")]
const VEGA_SOURCE: &str = include_str!("../bundles/vega/vega.min.js");

#[cfg(feature = "vega")]
const VEGA: Builtin = Builtin {
    name: "vega",
    source: VEGA_SOURCE,
    entry: "vegaEntry",
};

#[cfg(feature = "vega")]
const VEGA_LITE: Builtin = Builtin {
    name: "vega-lite",
    source: VEGA_SOURCE,
    entry: "vegaLiteEntry",
};

/// Every built-in renderer in this build, sorted by name.
const ALL: &[Builtin] = &[
    #[cfg(feature = "echarts")]
    ECHARTS,
    #[cfg(feature = "vega")]
    VEGA,
    #[cfg(feature = "vega")]
    VEGA_LITE,
];

/// The built-in renderer called `name`, if this build has it.
pub(crate) fn find(name: &str) -> Option<Builtin> {
    ALL.iter().find(|b| b.name == name).copied()
}

/// Names of the built-in renderers in this build, sorted.
///
/// Which ones are present depends on Cargo features, both on by default:
/// `echarts` adds `"echarts"`, and `vega` adds `"vega"` and `"vega-lite"`.
///
/// - **`echarts`** (Apache ECharts 6.1.0): the spec is an ECharts option
///   object. It is drawn with ECharts' server-side SVG renderer at the
///   island's width and height, with animation off and the `EN` locale.
/// - **`vega`** (Vega 6.4.0) and **`vega-lite`** (Vega-Lite 6.5.0): the spec
///   is a Vega or Vega-Lite spec and keeps its own size, except that a
///   Vega-Lite `"container"` width or height means the island's. Data must
///   be inline (`values`): a data `url`, an image mark or a link (`href`)
///   fails the render with an error naming it. Features that draw to a
///   raster canvas (the label transform, word clouds, heatmap images) are
///   not supported.
///
/// All of them measure text with the runner's fonts. Charts with thousands
/// of marks can take a second or more (a 2,000-point Vega-Lite scatter
/// takes 0.5 to 1.2 s in a release build), so they may need a time limit
/// above the default 2 s. A bundle registered under the same name with
/// [`IslandRunner::register_bundle`](crate::IslandRunner::register_bundle)
/// takes precedence over the built-in.
pub fn builtin_renderers() -> Vec<&'static str> {
    ALL.iter().map(|b| b.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_sorted_and_unique() {
        let names = builtin_renderers();
        assert!(names.windows(2).all(|w| w[0] < w[1]), "{names:?}");
    }

    #[test]
    fn every_listed_builtin_is_found() {
        for name in builtin_renderers() {
            let b = find(name).unwrap();
            assert_eq!(b.name, name);
            assert!(!b.source.is_empty());
        }
        assert!(find("no-such-renderer").is_none());
    }

    #[cfg(feature = "echarts")]
    #[test]
    fn echarts_is_built_in() {
        assert!(builtin_renderers().contains(&"echarts"));
        assert!(
            find("echarts")
                .unwrap()
                .source
                .contains("Apache ECharts 6.1.0")
        );
    }

    #[cfg(feature = "vega")]
    #[test]
    fn vega_and_vega_lite_are_built_in() {
        let names = builtin_renderers();
        assert!(names.contains(&"vega") && names.contains(&"vega-lite"));
        let vega = find("vega").unwrap();
        let lite = find("vega-lite").unwrap();
        assert!(vega.source == lite.source, "one shared bundle");
        assert_ne!(vega.entry, lite.entry);
        assert!(vega.source.contains("Vega 6.4.0 and Vega-Lite 6.5.0"));
    }
}
