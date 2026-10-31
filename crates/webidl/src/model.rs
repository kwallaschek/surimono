//! Many parsed IDL files, merged: partial interfaces and `includes`d mixins
//! are folded into one view per interface, in an order that depends only on
//! the files' names and contents, never on the order they were given in.

use std::collections::BTreeMap;

use crate::ParseError;
use crate::ast::*;

/// An error building an [`IdlSet`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SetError {
    /// A file failed to parse.
    #[error("{file}:{error}")]
    Parse {
        /// File name.
        file: String,
        /// The syntax error.
        error: ParseError,
    },
    /// The same file name was given twice.
    #[error("{0}: given twice")]
    DuplicateFile(String),
}

/// What a top-level name refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    /// An `interface`.
    Interface,
    /// An `interface mixin`.
    Mixin,
    /// A `callback interface`.
    CallbackInterface,
    /// A `namespace`.
    Namespace,
    /// A `dictionary`.
    Dictionary,
    /// An `enum`.
    Enum,
    /// A `typedef`.
    Typedef,
    /// A `callback` function.
    Callback,
}

/// The main definition of an interface (file, definition, interface) and its
/// partials (file, interface).
type Parts<'s> = (
    Option<(&'s str, &'s Definition, &'s Interface)>,
    Vec<(&'s str, &'s Interface)>,
);

/// Parsed IDL files, sorted by file name.
#[derive(Debug, Clone)]
pub struct IdlSet {
    files: Vec<(String, Vec<Definition>)>,
}

/// A member of a merged interface and the file and definition it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedMember {
    /// File the member was declared in.
    pub file: String,
    /// The interface or mixin that declared it (differs from the merged
    /// interface's name for mixin members).
    pub declared_in: String,
    /// The member.
    pub member: Member,
}

/// One interface with its partials and mixins folded in.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedInterface {
    /// Interface name.
    pub name: String,
    /// Inherited interface, if any.
    pub parent: Option<String>,
    /// Extended attributes of the main (non-partial) definition.
    pub ext_attrs: Vec<ExtAttr>,
    /// File holding the main definition.
    pub file: String,
    /// Every file that contributed members, sorted, without duplicates.
    pub files: Vec<String>,
    /// Mixins included, in merge order.
    pub mixins: Vec<String>,
    /// Members: the main definition's, then partials' (by file name, then
    /// source order), then each mixin's (by the `includes` statement's file
    /// name and position; a mixin's own partials follow it the same way).
    pub members: Vec<MergedMember>,
}

impl IdlSet {
    /// Parses `(file name, source)` pairs. The result is the same whatever
    /// order the pairs come in.
    pub fn parse_sources<'a>(
        sources: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, SetError> {
        let mut files = Vec::new();
        for (name, src) in sources {
            let defs = crate::parse(src).map_err(|error| SetError::Parse {
                file: name.to_owned(),
                error,
            })?;
            files.push((name.to_owned(), defs));
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        if let Some(w) = files.windows(2).find(|w| w[0].0 == w[1].0) {
            return Err(SetError::DuplicateFile(w[0].0.clone()));
        }
        Ok(IdlSet { files })
    }

    /// The parsed files, sorted by name.
    pub fn files(&self) -> &[(String, Vec<Definition>)] {
        &self.files
    }

    fn definitions(&self) -> impl Iterator<Item = (&str, &Definition)> {
        self.files
            .iter()
            .flat_map(|(f, defs)| defs.iter().map(move |d| (f.as_str(), d)))
    }

    /// Every non-partial top-level name and what it is. A name defined more
    /// than once keeps its first definition (by file name).
    pub fn names(&self) -> BTreeMap<String, NameKind> {
        let mut out = BTreeMap::new();
        for (_, d) in self.definitions() {
            let (name, kind) = match &d.kind {
                DefinitionKind::Interface(i) if !i.partial => (
                    &i.name,
                    match i.kind {
                        InterfaceKind::Interface => NameKind::Interface,
                        InterfaceKind::Mixin => NameKind::Mixin,
                        InterfaceKind::Callback => NameKind::CallbackInterface,
                    },
                ),
                DefinitionKind::Namespace(n) if !n.partial => (&n.name, NameKind::Namespace),
                DefinitionKind::Dictionary(x) if !x.partial => (&x.name, NameKind::Dictionary),
                DefinitionKind::Enum { name, .. } => (name, NameKind::Enum),
                DefinitionKind::Typedef { name, .. } => (name, NameKind::Typedef),
                DefinitionKind::Callback { name, .. } => (name, NameKind::Callback),
                _ => continue,
            };
            out.entry(name.clone()).or_insert(kind);
        }
        out
    }

    /// Names defined (non-partially) more than once, with their files.
    pub fn duplicate_names(&self) -> Vec<(String, Vec<String>)> {
        let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (f, d) in self.definitions() {
            let name = match &d.kind {
                DefinitionKind::Interface(i) if !i.partial => &i.name,
                DefinitionKind::Namespace(n) if !n.partial => &n.name,
                DefinitionKind::Dictionary(x) if !x.partial => &x.name,
                DefinitionKind::Enum { name, .. }
                | DefinitionKind::Typedef { name, .. }
                | DefinitionKind::Callback { name, .. } => name,
                _ => continue,
            };
            seen.entry(name.clone()).or_default().push(f.to_owned());
        }
        seen.into_iter().filter(|(_, v)| v.len() > 1).collect()
    }

    /// The type a typedef stands for.
    pub fn typedef(&self, name: &str) -> Option<&Type> {
        self.definitions().find_map(|(_, d)| match &d.kind {
            DefinitionKind::Typedef { ty, name: n } if n == name => Some(ty),
            _ => None,
        })
    }

    /// Interfaces or mixins named `name` of `kind`: the main definition (if
    /// any) first, then partials in file-name and source order.
    fn interface_parts(&self, name: &str, kind: InterfaceKind) -> Parts<'_> {
        let mut main = None;
        let mut partials = Vec::new();
        for (f, d) in self.definitions() {
            if let DefinitionKind::Interface(i) = &d.kind {
                if i.name == name && i.kind == kind {
                    if i.partial {
                        partials.push((f, i));
                    } else if main.is_none() {
                        main = Some((f, d, i));
                    }
                }
            }
        }
        (main, partials)
    }

    /// The interface `name` with partials and mixins merged, or `None` if no
    /// non-partial `interface` of that name exists.
    pub fn interface(&self, name: &str) -> Option<MergedInterface> {
        let (main, partials) = self.interface_parts(name, InterfaceKind::Interface);
        let (file, def, iface) = main?;
        let mut members = Vec::new();
        let mut push = |f: &str, declared_in: &str, ms: &[Member]| {
            for m in ms {
                members.push(MergedMember {
                    file: f.to_owned(),
                    declared_in: declared_in.to_owned(),
                    member: m.clone(),
                });
            }
        };
        push(file, name, &iface.members);
        for (f, p) in &partials {
            push(f, name, &p.members);
        }
        let mut mixins = Vec::new();
        for (_, d) in self.definitions() {
            if let DefinitionKind::Includes { target, mixin } = &d.kind {
                if target == name && !mixins.contains(mixin) {
                    mixins.push(mixin.clone());
                }
            }
        }
        for mixin in &mixins {
            let (mmain, mpartials) = self.interface_parts(mixin, InterfaceKind::Mixin);
            if let Some((f, _, m)) = mmain {
                push(f, mixin, &m.members);
            }
            for (f, p) in &mpartials {
                push(f, mixin, &p.members);
            }
        }
        let mut files: Vec<String> = members.iter().map(|m| m.file.clone()).collect();
        files.push(file.to_owned());
        files.sort();
        files.dedup();
        Some(MergedInterface {
            name: name.to_owned(),
            parent: iface.inherits.clone(),
            ext_attrs: def.ext_attrs.clone(),
            file: file.to_owned(),
            files,
            mixins,
            members,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "interface I : P { attribute long a; }; I includes M; partial interface I { undefined b(); };";
    const B: &str = "interface mixin M { readonly attribute long m; }; partial interface I { const long C = 1; }; partial interface mixin M { undefined n(); };";

    fn names(i: &MergedInterface) -> Vec<String> {
        i.members.iter().map(|m| m.member.to_string()).collect()
    }

    #[test]
    fn merge_order_is_by_file_name_not_input_order() {
        let x = IdlSet::parse_sources([("a.idl", A), ("b.idl", B)]).unwrap();
        let y = IdlSet::parse_sources([("b.idl", B), ("a.idl", A)]).unwrap();
        let (ix, iy) = (x.interface("I").unwrap(), y.interface("I").unwrap());
        assert_eq!(ix, iy);
        assert_eq!(
            names(&ix),
            [
                "attribute long a;",
                "undefined b();",
                "const long C = 1;",
                "readonly attribute long m;",
                "undefined n();",
            ]
        );
        assert_eq!(ix.parent.as_deref(), Some("P"));
        assert_eq!(ix.mixins, ["M"]);
        assert_eq!(ix.files, ["a.idl", "b.idl"]);
        assert_eq!(ix.members[3].declared_in, "M");
        assert!(x.interface("M").is_none());
        assert_eq!(x.names().get("M"), Some(&NameKind::Mixin));
    }

    #[test]
    fn errors_name_the_file() {
        let e = IdlSet::parse_sources([("bad.idl", "interface")]).unwrap_err();
        assert!(e.to_string().starts_with("bad.idl:1:"), "{e}");
        let e = IdlSet::parse_sources([("a.idl", ""), ("a.idl", "")]).unwrap_err();
        assert_eq!(e, SetError::DuplicateFile("a.idl".into()));
    }
}
