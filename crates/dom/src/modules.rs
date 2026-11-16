//! SPIKE (P2-RT-12): import maps and module specifier resolution (HTML
//! §8.1.5 "Import maps", "resolve a module specifier"). Not for merging as is.

use url::Url;

/// The base a page without a URL of its own resolves against. `.invalid`
/// can't be registered (RFC 6761), so it is never a real host; the loader is
/// given the path and query of a URL here, as written in a Rails page.
pub(crate) const SYNTHETIC_ORIGIN: &str = "https://surimono.invalid";

/// A specifier map, sorted so that longer keys come first (code unit order,
/// descending). A `None` value is a key the map blocks.
type SpecifierMap = Vec<(String, Option<Url>)>;

/// A merged import map.
#[derive(Debug, Default)]
pub(crate) struct ImportMap {
    imports: SpecifierMap,
    scopes: Vec<(String, SpecifierMap)>,
    /// Warnings from parsing, as a console would show them.
    pub(crate) warnings: Vec<String>,
}

/// HTML "parse a URL-like import specifier".
fn url_like(specifier: &str, base: &Url) -> Option<Url> {
    if specifier.starts_with('/') || specifier.starts_with("./") || specifier.starts_with("../") {
        return base.join(specifier).ok();
    }
    Url::parse(specifier).ok()
}

fn sort_desc(map: &mut SpecifierMap) {
    map.sort_by(|a, b| b.0.encode_utf16().cmp(a.0.encode_utf16()));
}

fn specifier_map(
    obj: &serde_json::Map<String, serde_json::Value>,
    base: &Url,
    warnings: &mut Vec<String>,
) -> SpecifierMap {
    let mut out = Vec::new();
    for (key, value) in obj {
        if key.is_empty() {
            warnings.push("empty specifier key".into());
            continue;
        }
        let normalized = url_like(key, base).map_or_else(|| key.clone(), String::from);
        let Some(text) = value.as_str() else {
            warnings.push(format!("the address of {key:?} is not a string"));
            out.push((normalized, None));
            continue;
        };
        let address = url_like(text, base);
        let address = match address {
            Some(a) if key.ends_with('/') && !a.as_str().ends_with('/') => {
                warnings.push(format!("{key:?} ends with / but its address doesn't"));
                None
            }
            Some(a) => Some(a),
            None => {
                warnings.push(format!("the address of {key:?} is not a URL"));
                None
            }
        };
        out.push((normalized, address));
    }
    sort_desc(&mut out);
    out
}

impl ImportMap {
    /// HTML "parse an import map string" with `base` the document's base URL,
    /// merged into `self` with the earlier map's entries winning (HTML "merge
    /// existing and new import maps", without dropping already-resolved keys).
    pub(crate) fn add(&mut self, text: &str, base: &Url) -> Result<(), String> {
        let json: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("SyntaxError: import map: {e}"))?;
        let top = json
            .as_object()
            .ok_or("TypeError: an import map must be a JSON object")?;
        let mut warnings = Vec::new();
        let imports = match top.get("imports") {
            Some(v) => specifier_map(
                v.as_object()
                    .ok_or("TypeError: \"imports\" must be an object")?,
                base,
                &mut warnings,
            ),
            None => Vec::new(),
        };
        let mut scopes = Vec::new();
        if let Some(v) = top.get("scopes") {
            for (prefix, map) in v
                .as_object()
                .ok_or("TypeError: \"scopes\" must be an object")?
            {
                let map = map
                    .as_object()
                    .ok_or_else(|| format!("TypeError: the scope {prefix:?} must be an object"))?;
                let Ok(url) = base.join(prefix) else {
                    warnings.push(format!("the scope prefix {prefix:?} is not a URL"));
                    continue;
                };
                scopes.push((String::from(url), specifier_map(map, base, &mut warnings)));
            }
        }
        if top.contains_key("integrity") {
            warnings.push("\"integrity\" is ignored: no network, no fetch".into());
        }
        for (k, v) in imports {
            if !self.imports.iter().any(|(e, _)| *e == k) {
                self.imports.push((k, v));
            }
        }
        sort_desc(&mut self.imports);
        for (prefix, map) in scopes {
            match self.scopes.iter_mut().find(|(p, _)| *p == prefix) {
                Some((_, existing)) => {
                    for (k, v) in map {
                        if !existing.iter().any(|(e, _)| *e == k) {
                            existing.push((k, v));
                        }
                    }
                    sort_desc(existing);
                }
                None => self.scopes.push((prefix, map)),
            }
        }
        self.scopes
            .sort_by(|a, b| b.0.encode_utf16().cmp(a.0.encode_utf16()));
        self.warnings.extend(warnings);
        Ok(())
    }

    /// HTML "resolve a module specifier" for a referrer at `base`.
    pub(crate) fn resolve(&self, base: &Url, specifier: &str) -> Result<Url, String> {
        let as_url = url_like(specifier, base);
        let normalized = as_url
            .as_ref()
            .map_or_else(|| specifier.to_owned(), |u| u.as_str().to_owned());
        let base_s = base.as_str();
        for (prefix, map) in &self.scopes {
            if prefix == base_s || (prefix.ends_with('/') && base_s.starts_with(prefix.as_str())) {
                if let Some(found) = imports_match(&normalized, as_url.as_ref(), map)? {
                    return Ok(found);
                }
            }
        }
        if let Some(found) = imports_match(&normalized, as_url.as_ref(), &self.imports)? {
            return Ok(found);
        }
        as_url.ok_or_else(|| {
            format!(
                "Failed to resolve module specifier \"{specifier}\". Relative references must start with either \"/\", \"./\", or \"../\"."
            )
        })
    }
}

/// HTML "resolve an imports match".
fn imports_match(
    normalized: &str,
    as_url: Option<&Url>,
    map: &SpecifierMap,
) -> Result<Option<Url>, String> {
    for (key, value) in map {
        if key == normalized {
            return value
                .clone()
                .map(Some)
                .ok_or_else(|| format!("the import map blocks \"{normalized}\""));
        }
        let special = as_url
            .is_none_or(|u| matches!(u.scheme(), "http" | "https" | "ftp" | "ws" | "wss" | "file"));
        if key.ends_with('/') && normalized.starts_with(key.as_str()) && special {
            let Some(value) = value else {
                return Err(format!("the import map blocks \"{normalized}\""));
            };
            let after = &normalized[key.len()..];
            let url = value
                .join(after)
                .map_err(|_| format!("\"{normalized}\" doesn't resolve under {value}"))?;
            if !url.as_str().starts_with(value.as_str()) {
                return Err(format!("\"{normalized}\" backtracks above {value}"));
            }
            return Ok(Some(url));
        }
    }
    Ok(None)
}

/// The URL the loader is given for `url`: its path and query on the
/// synthetic origin (as a Rails or Vite page writes it), else the whole URL.
pub(crate) fn loader_url(url: &str) -> String {
    match url.strip_prefix(SYNTHETIC_ORIGIN) {
        Some(rest) if rest.starts_with('/') => rest.split('#').next().unwrap_or(rest).to_owned(),
        _ => url.split('#').next().unwrap_or(url).to_owned(),
    }
}

/// The document's base for module resolution: its base URL if that is
/// hierarchical, else the synthetic origin.
pub(crate) fn module_base(doc_base: &str) -> Url {
    match Url::parse(doc_base) {
        Ok(u) if !u.cannot_be_a_base() => u,
        _ => Url::parse(&format!("{SYNTHETIC_ORIGIN}/")).expect("a constant URL"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rails_map_resolves() {
        let base = module_base("about:blank");
        let mut m = ImportMap::default();
        m.add(
            r#"{"imports":{"application":"/assets/application-1.js","controllers":"/assets/controllers/index-2.js","controllers/hello_controller":"/assets/controllers/hello_controller-3.js","lodash/":"/vendor/lodash/"}}"#,
            &base,
        )
        .unwrap();
        let r = |s: &str| m.resolve(&base, s).map(|u| loader_url(u.as_str()));
        assert_eq!(r("application").unwrap(), "/assets/application-1.js");
        assert_eq!(r("controllers").unwrap(), "/assets/controllers/index-2.js");
        assert_eq!(
            r("controllers/hello_controller").unwrap(),
            "/assets/controllers/hello_controller-3.js"
        );
        assert_eq!(r("lodash/map.js").unwrap(), "/vendor/lodash/map.js");
        assert_eq!(r("./x.js").unwrap(), "/x.js");
        assert!(r("react").is_err());
    }
}
