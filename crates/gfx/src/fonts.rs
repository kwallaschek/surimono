//! The font registry: caller-supplied font files only, never system fonts.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use parley::fontique::{Blob, Collection, CollectionOptions, FamilyId};
use skrifa::raw::FileRef;

use crate::GfxError;
use crate::css_font::{FamilyEntry, GenericFamily};

/// A set of fonts registered from bytes the caller supplies.
///
/// System fonts are never consulted. Generic families (`sans-serif`, `serif`,
/// `monospace`, ...) map to the first registered family unless set with
/// [`FontSet::set_generic_family`]. Give fulgur the same font files (see
/// [`FontSet::font_files`]) so measured text matches printed text.
#[derive(Clone)]
pub struct FontSet {
    pub(crate) collection: Collection,
    /// Family names in registration order, as the fonts spell them.
    families: Vec<String>,
    /// Explicit generic family mappings.
    generics: BTreeMap<GenericFamily, Vec<String>>,
    /// The registered files, in registration order.
    files: Vec<Arc<Vec<u8>>>,
    faces: usize,
}

impl fmt::Debug for FontSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontSet")
            .field("families", &self.families)
            .field("generics", &self.generics)
            .field("files", &self.files.len())
            .field("faces", &self.faces)
            .finish()
    }
}

impl Default for FontSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Font container formats, detected from the first four bytes.
fn container(bytes: &[u8]) -> Result<(), GfxError> {
    match bytes.get(..4) {
        Some(b"OTTO" | b"ttcf" | [0x00, 0x01, 0x00, 0x00] | b"true" | b"typ1") => Ok(()),
        Some(b"wOF2") => Err(GfxError::UnsupportedFontFormat { format: "WOFF2" }),
        Some(b"wOFF") => Err(GfxError::UnsupportedFontFormat { format: "WOFF" }),
        _ => Err(GfxError::InvalidFont {
            reason: "unknown file signature (expected TTF, OTF or TTC)".into(),
        }),
    }
}

impl FontSet {
    /// An empty font set.
    pub fn new() -> Self {
        FontSet {
            collection: Collection::new(CollectionOptions {
                shared: false,
                system_fonts: false,
            }),
            families: Vec::new(),
            generics: BTreeMap::new(),
            files: Vec::new(),
            faces: 0,
        }
    }

    /// Registers every face in a TTF, OTF or TTC file and returns how many
    /// faces were added. WOFF and WOFF2 are rejected with
    /// [`GfxError::UnsupportedFontFormat`]; decode them first.
    pub fn add_font_bytes(&mut self, bytes: Vec<u8>) -> Result<usize, GfxError> {
        container(&bytes)?;
        // Validate with skrifa first so a broken file is an error, not a silent no-op.
        let file = FileRef::new(&bytes).map_err(|e| GfxError::InvalidFont {
            reason: e.to_string(),
        })?;
        let parsed = file.fonts().filter(|f| f.is_ok()).count();
        if parsed == 0 {
            return Err(GfxError::InvalidFont {
                reason: "no readable font face in the file".into(),
            });
        }

        let data = Arc::new(bytes);
        let blob = Blob::new(data.clone());
        let mut added = self.collection.register_fonts(blob, None);
        let faces: usize = added.iter().map(|(_, fonts)| fonts.len()).sum();
        if faces == 0 {
            return Err(GfxError::InvalidFont {
                reason: "no face has a usable family name".into(),
            });
        }
        // Fontique returns families in hash order; order them by their first
        // face in the file so registration order is deterministic.
        added.sort_by_key(|(_, fonts)| fonts.iter().map(|f| f.index()).min());
        for (id, _) in &added {
            self.push_family(*id);
        }
        self.files.push(data);
        self.faces += faces;
        Ok(faces)
    }

    fn push_family(&mut self, id: FamilyId) {
        let Some(name) = self.collection.family_name(id).map(str::to_string) else {
            return;
        };
        if self.canonical(&name).is_none() {
            self.families.push(name);
        }
    }

    /// Registered family names, in registration order.
    pub fn families(&self) -> Vec<String> {
        self.families.clone()
    }

    /// Number of faces registered so far.
    pub fn face_count(&self) -> usize {
        self.faces
    }

    /// The registered font files, unchanged and in registration order. Pass
    /// these same bytes to the PDF renderer (for fulgur,
    /// `AssetBundle::add_font_bytes`) so it prints with the fonts we measured.
    pub fn font_files(&self) -> impl Iterator<Item = &[u8]> + '_ {
        self.files.iter().map(|f| f.as_slice())
    }

    /// Maps a generic family to registered families, tried in order.
    ///
    /// Fails with [`GfxError::UnknownFamily`] if a name is not registered.
    /// An empty list restores the default (the first registered family).
    pub fn set_generic_family(
        &mut self,
        generic: GenericFamily,
        families: &[&str],
    ) -> Result<(), GfxError> {
        let mut resolved = Vec::with_capacity(families.len());
        for name in families {
            let canonical = self
                .canonical(name)
                .ok_or_else(|| GfxError::UnknownFamily {
                    family: (*name).to_string(),
                })?;
            if !resolved.iter().any(|r: &String| r == canonical) {
                resolved.push(canonical.to_string());
            }
        }
        if resolved.is_empty() {
            self.generics.remove(&generic);
        } else {
            self.generics.insert(generic, resolved);
        }
        Ok(())
    }

    /// The registered spelling of a family name (ASCII case-insensitive).
    fn canonical(&self, name: &str) -> Option<&str> {
        self.families
            .iter()
            .find(|f| f.eq_ignore_ascii_case(name))
            .map(String::as_str)
    }

    /// Families a generic maps to.
    fn generic(&self, generic: GenericFamily) -> &[String] {
        match self.generics.get(&generic) {
            Some(list) => list,
            None => &self.families[..self.families.len().min(1)],
        }
    }

    /// Resolves a parsed family list to the registered families to try, in
    /// order, the way measurement and the SVG rewrite both use it:
    /// 1. named families that are registered, and generics expanded to their
    ///    mapping (unknown names are skipped, as browsers skip missing fonts);
    /// 2. if nothing matched, the `sans-serif` mapping (canvas's default);
    /// 3. every other registered family, in registration order, as per
    ///    character fallback (so Japanese in a Latin list finds a CJK font).
    pub(crate) fn resolve_stack(&self, entries: &[FamilyEntry]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let push = |out: &mut Vec<String>, name: &str| {
            if !out.iter().any(|o| o == name) {
                out.push(name.to_string());
            }
        };
        for entry in entries {
            match entry {
                FamilyEntry::Named(name) => {
                    if let Some(c) = self.canonical(name) {
                        push(&mut out, c);
                    }
                }
                FamilyEntry::Generic(g) => {
                    for name in self.generic(*g) {
                        push(&mut out, name);
                    }
                }
            }
        }
        if out.is_empty() {
            for name in self.generic(GenericFamily::SansSerif) {
                push(&mut out, name);
            }
        }
        for name in &self.families {
            push(&mut out, name);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_fonts_without_panicking() {
        let mut set = FontSet::new();
        for bytes in [
            Vec::new(),
            b"abc".to_vec(),
            b"hello world, not a font".to_vec(),
            vec![0x00, 0x01, 0x00, 0x00],
            vec![
                0x00, 0x01, 0x00, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
            b"OTTO\0\0\0\0\0\0\0\0".to_vec(),
            b"ttcf\0\x01\0\0\0\0\0\x05".to_vec(),
        ] {
            assert!(
                matches!(
                    set.add_font_bytes(bytes.clone()),
                    Err(GfxError::InvalidFont { .. })
                ),
                "{bytes:?}"
            );
        }
        assert_eq!(
            set.add_font_bytes(b"wOF2\0\0\0\0".to_vec()),
            Err(GfxError::UnsupportedFontFormat { format: "WOFF2" })
        );
        assert_eq!(
            set.add_font_bytes(b"wOFF\0\0\0\0".to_vec()),
            Err(GfxError::UnsupportedFontFormat { format: "WOFF" })
        );
        assert!(set.families().is_empty());
        assert_eq!(set.face_count(), 0);
    }

    #[test]
    fn unknown_generic_target_is_an_error() {
        let mut set = FontSet::new();
        assert_eq!(
            set.set_generic_family(GenericFamily::Serif, &["Nope"]),
            Err(GfxError::UnknownFamily {
                family: "Nope".into()
            })
        );
    }

    #[test]
    fn empty_set_resolves_to_nothing() {
        let set = FontSet::new();
        assert!(
            set.resolve_stack(&[FamilyEntry::Generic(GenericFamily::SansSerif)])
                .is_empty()
        );
    }
}
