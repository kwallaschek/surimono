//! What HTML defines over the tree and no other standard does: the lists tables,
//! forms and selects keep, and `innerText`. Members that only move nodes are in
//! `glue_html.js`.

use blitz_dom::ns;
use surimono_webidl::runtime::BindingResult;

use crate::document::{Document, Kind, ListKind};
use crate::style_attr;

/// Elements whose text isn't rendered, by the HTML rendering section's
/// `display: none` rules and the replaced controls, whose text is no box's.
const UNRENDERED: &[&str] = &[
    "area", "audio", "base", "basefont", "canvas", "datalist", "embed", "head", "iframe", "input",
    "link", "meta", "meter", "noembed", "noframes", "noscript", "object", "param", "progress",
    "rp", "script", "select", "style", "template", "textarea", "title", "video",
];

/// Elements that are blocks by the rendering section.
const BLOCKS: &[&str] = &[
    "address", "article", "aside", "blockquote", "body", "caption", "center", "dd", "details",
    "dialog", "dir", "div", "dl", "dt", "fieldset", "figcaption", "figure", "footer", "form", "h1",
    "h2", "h3", "h4", "h5", "h6", "header", "hgroup", "hr", "html", "legend", "li", "listing",
    "main", "menu", "nav", "ol", "plaintext", "pre", "search", "section", "summary", "table", "ul",
    "xmp",
];

/// Elements whose text keeps its white space, by the same section.
const PRE: &[&str] = &["listing", "plaintext", "pre", "xmp"];

/// One item of the "rendered text collection".
enum Piece {
    /// Text whose white space has collapsed.
    Text(String),
    /// Text that keeps it.
    Pre(String),
    /// A `\n` or `\t` the standard writes out.
    Hard(char),
    /// A required line break count.
    Lines(u8),
}

impl Document {
    /// `n`'s local name if it is an HTML element.
    pub(crate) fn html_local(&self, n: usize) -> Option<&str> {
        self.element_name(n)
            .filter(|e| e.ns == ns!(html))
            .map(|e| e.local.as_ref())
    }

    fn html_children(&self, owner: usize, names: &[&str]) -> Vec<usize> {
        self.children(owner)
            .iter()
            .copied()
            .filter(|&c| self.html_local(c).is_some_and(|l| names.contains(&l)))
            .collect()
    }

    /// The lists of HTML §4.9, §4.10 and §3.1.3.
    pub(crate) fn html_list(&self, owner: usize, kind: &ListKind) -> BindingResult<Vec<usize>> {
        let named = |names: &'static [&'static str]| {
            self.descendants(owner, move |n| {
                Ok(self.html_local(n).is_some_and(|l| names.contains(&l)))
            })
        };
        Ok(match kind {
            ListKind::Forms => named(&["form"])?,
            ListKind::Images => named(&["img"])?,
            ListKind::Embeds => named(&["embed"])?,
            ListKind::Links => self.descendants(owner, |n| {
                Ok(matches!(self.html_local(n), Some("a" | "area"))
                    && self.attribute(n, "href").is_some())
            })?,
            ListKind::Anchors => self.descendants(owner, |n| {
                Ok(self.html_local(n) == Some("a") && self.attribute(n, "name").is_some())
            })?,
            ListKind::TableRows => self.table_rows(owner),
            ListKind::TableBodies => self.html_children(owner, &["tbody"]),
            ListKind::SectionRows => self.html_children(owner, &["tr"]),
            ListKind::RowCells => self.html_children(owner, &["td", "th"]),
            ListKind::Options => self.options_of(owner),
            ListKind::SelectedOptions => self.selected_options(owner),
            ListKind::FormControls => self.form_controls(owner)?,
            _ => Vec::new(),
        })
    }

    /// §4.9.1 `rows`: the head's, the body's in tree order, then the foot's.
    fn table_rows(&self, table: usize) -> Vec<usize> {
        let of = |names: &[&str]| -> Vec<usize> {
            self.html_children(table, names)
                .into_iter()
                .flat_map(|s| self.html_children(s, &["tr"]))
                .collect()
        };
        let mut rows = of(&["thead"]);
        for c in self.html_children(table, &["tr", "tbody"]) {
            if self.html_local(c) == Some("tr") {
                rows.push(c);
            } else {
                rows.extend(self.html_children(c, &["tr"]));
            }
        }
        rows.extend(of(&["tfoot"]));
        rows
    }

    /// §4.10.7 "list of options".
    fn options_of(&self, select: usize) -> Vec<usize> {
        let mut out = Vec::new();
        for c in self.html_children(select, &["option", "optgroup"]) {
            if self.html_local(c) == Some("option") {
                out.push(c);
            } else {
                out.extend(self.html_children(c, &["option"]));
            }
        }
        out
    }

    fn option_disabled(&self, option: usize) -> bool {
        self.attribute(option, "disabled").is_some()
            || self.parent(option).is_some_and(|p| {
                self.html_local(p) == Some("optgroup") && self.attribute(p, "disabled").is_some()
            })
    }

    /// A select's selected options. `selected` is the attribute, as `checked` is
    /// for an input, because what a script chooses has to reach the printed page.
    /// §4.10.7's "selectedness setting algorithm" over that: a single select shows
    /// its last selected option, or else, at display size 1, its first enabled one.
    pub(crate) fn selected_options(&self, select: usize) -> Vec<usize> {
        let options = self.options_of(select);
        let mut chosen: Vec<usize> = options
            .iter()
            .copied()
            .filter(|&o| self.attribute(o, "selected").is_some())
            .collect();
        if self.attribute(select, "multiple").is_some() {
            return chosen;
        }
        if chosen.len() > 1 {
            chosen.drain(..chosen.len() - 1);
        }
        let size = self
            .attribute(select, "size")
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        if chosen.is_empty() && size <= 1 {
            chosen.extend(options.iter().find(|&&o| !self.option_disabled(o)));
        }
        chosen
    }

    /// §4.10.17.3 "form owner": what `form` names if connected, otherwise the
    /// nearest form ancestor.
    fn form_owner(&self, n: usize) -> Option<usize> {
        if let Some(id) = self.attribute(n, "form")
            && self.is_connected(n)
        {
            let top = self.owner_of_node(n);
            return self
                .element_by_id(top, &id)
                .filter(|&f| self.html_local(f) == Some("form"));
        }
        let mut at = self.parent(n);
        while let Some(p) = at {
            if self.html_local(p) == Some("form") {
                return Some(p);
            }
            at = self.parent(p);
        }
        None
    }

    /// §4.10.3 `elements`: the listed elements whose form owner is `form`, except
    /// image buttons, in tree order, under the form's root.
    fn form_controls(&self, form: usize) -> BindingResult<Vec<usize>> {
        let mut top = form;
        while let Some(p) = self.parent(top) {
            top = p;
        }
        self.descendants(top, |n| {
            let listed = match self.html_local(n) {
                Some("button" | "fieldset" | "object" | "output" | "select" | "textarea") => true,
                Some("input") => !self
                    .attribute(n, "type")
                    .is_some_and(|t| t.eq_ignore_ascii_case("image")),
                _ => false,
            };
            Ok(listed && self.form_owner(n) == Some(form))
        })
    }

    /// Whether an inline style says `display: none`. An attribute that can't be
    /// parsed in time says nothing, and the time limit ends the script anyway.
    fn inline_none(&self, n: usize) -> bool {
        let Some(text) = self.attribute(n, "style") else {
            return false;
        };
        let list = style_attr::parse(&text, self.quirks_mode(), self.deadline.as_ref());
        list.is_ok_and(|l| style_attr::value(&l, "display") == "none")
    }

    fn unrendered(&self, n: usize) -> bool {
        self.attribute(n, "hidden").is_some()
            || self.inline_none(n)
            || self.html_local(n).is_some_and(|l| UNRENDERED.contains(&l))
    }

    /// HTML §3.2.7 `innerText`, with nothing laid out. An element out of the tree
    /// isn't being rendered, so it gets its `textContent`, as the standard says.
    /// Otherwise "rendered text collection", with the display of each element from
    /// the rendering section's defaults, the `hidden` attribute and an inline
    /// `display: none`: a stylesheet's rules aren't seen until phase 3.
    pub(crate) fn inner_text(&self, id: usize) -> BindingResult<String> {
        if !self.is_connected(id) || self.unrendered(id) {
            return Ok(self.text_content(id).unwrap_or_default());
        }
        let mut pieces = Vec::new();
        // `false` is a first visit, `true` the way out.
        let mut stack = vec![(id, false)];
        while let Some((n, leaving)) = stack.pop() {
            self.poll()?;
            let local = self.html_local(n).unwrap_or_default();
            if leaving {
                match local {
                    "p" => pieces.push(Piece::Lines(2)),
                    "td" | "th" if self.next_element(n).is_some() => {
                        pieces.push(Piece::Hard('\t'));
                    }
                    "tr" if self.next_element(n).is_some() => {
                        pieces.push(Piece::Hard('\n'));
                    }
                    l if BLOCKS.contains(&l) => pieces.push(Piece::Lines(1)),
                    _ => {}
                }
                continue;
            }
            match self.kind(n) {
                Some(Kind::Text) => {
                    let text = self.char_data(n);
                    let pre = self.in_preformatted(n);
                    pieces.push(if pre {
                        Piece::Pre(text.to_owned())
                    } else {
                        Piece::Text(collapse(text))
                    });
                }
                Some(Kind::Element) if n != id && self.unrendered(n) => {}
                Some(Kind::Element) => {
                    if local == "br" {
                        pieces.push(Piece::Hard('\n'));
                        continue;
                    }
                    if local == "p" {
                        pieces.push(Piece::Lines(2));
                    } else if BLOCKS.contains(&local) {
                        pieces.push(Piece::Lines(1));
                    }
                    stack.push((n, true));
                    stack.extend(self.children(n).iter().rev().map(|&c| (c, false)));
                }
                _ => {}
            }
        }
        Ok(assemble(pieces))
    }

    fn next_element(&self, n: usize) -> Option<usize> {
        let parent = self.parent(n)?;
        let siblings = self.children(parent);
        let at = siblings.iter().position(|&c| c == n)?;
        siblings[at + 1..]
            .iter()
            .copied()
            .find(|&c| self.kind(c) == Some(Kind::Element))
    }

    fn in_preformatted(&self, n: usize) -> bool {
        let mut at = self.parent(n);
        while let Some(p) = at {
            if self.html_local(p).is_some_and(|l| PRE.contains(&l)) {
                return true;
            }
            at = self.parent(p);
        }
        false
    }

    /// §3.2.7 "rendered text fragment": line breaks become `br`, `\r\n` one.
    pub(crate) fn set_inner_text(&mut self, id: usize, text: &str) -> Result<(), crate::DomError> {
        if text.is_empty() {
            self.replace_all(id, None);
            return Ok(());
        }
        let fragment = self.create_fragment()?;
        let normal = text.replace("\r\n", "\n").replace('\r', "\n");
        for (i, line) in normal.split('\n').enumerate() {
            if i > 0
                && let Some(br) = self.create_element("br")?
            {
                self.place(fragment, &[br], None);
            }
            if !line.is_empty() {
                let t = self.create_text(line)?;
                self.place(fragment, &[t], None);
            }
        }
        self.replace_all_parsed(id, fragment);
        Ok(())
    }
}

/// CSS `white-space: normal` over one text node: runs become one space.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for c in text.chars() {
        let space = matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}');
        if !space {
            out.push(c);
        } else if !last_space {
            out.push(' ');
        }
        last_space = space;
    }
    out
}

/// The last steps of §3.2.7, and what layout does to spaces at the ends of lines: a
/// run of required breaks is its largest count of newlines, with none at the start
/// or the end.
fn assemble(pieces: Vec<Piece>) -> String {
    let mut out = String::new();
    let mut wanted = 0;
    let mut space = false;
    let mut line_start = true;
    let flush = |out: &mut String, wanted: &mut u8| {
        if !out.is_empty() {
            for _ in 0..*wanted {
                out.push('\n');
            }
        }
        *wanted = 0;
    };
    for piece in pieces {
        match piece {
            Piece::Lines(n) => {
                wanted = wanted.max(n);
                space = false;
                line_start = true;
            }
            Piece::Hard(c) => {
                flush(&mut out, &mut wanted);
                out.push(c);
                space = false;
                line_start = c == '\n';
            }
            Piece::Pre(text) => {
                if text.is_empty() {
                    continue;
                }
                flush(&mut out, &mut wanted);
                if space {
                    out.push(' ');
                }
                out.push_str(&text);
                space = false;
                line_start = text.ends_with('\n');
            }
            Piece::Text(text) => {
                for c in text.chars() {
                    if c == ' ' {
                        space = !line_start;
                        continue;
                    }
                    flush(&mut out, &mut wanted);
                    if space {
                        out.push(' ');
                    }
                    out.push(c);
                    space = false;
                    line_start = false;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_space_collapses_inside_a_run() {
        assert_eq!(collapse(" a \n\t b "), " a b ");
    }

    #[test]
    fn lines_are_the_largest_count() {
        let got = assemble(vec![
            Piece::Lines(2),
            Piece::Text("a ".to_owned()),
            Piece::Lines(1),
            Piece::Lines(2),
            Piece::Text(" b".to_owned()),
            Piece::Lines(1),
        ]);
        assert_eq!(got, "a\n\nb");
    }
}
