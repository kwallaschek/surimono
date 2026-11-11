//! A page's CSS as the printer-safe pass ([`super::safe`]) needs it: which
//! rules there are, what each declares, and which elements a rule's selector
//! could match.
//!
//! This reads CSS the way Chrome does, since the pass has to leave Chrome's
//! rendering alone: the raw text of a `<style>`, split into rules and
//! declarations by CSS Syntax's own steps on `cssparser` tokens. Nothing is
//! resolved. Every answer is one-sided instead: [`Subject::may_match`] says no
//! only when a type, class or id in the selector's last compound rules the
//! element out, and anything this reader doesn't know (a nested rule, a
//! keyframe, a namespace prefix, a bracket that never closes) counts as a rule
//! that matches everything. So "no rule that may match this element declares
//! `display`" is true of the page whatever else is.
//!
//! The reader also says when a page's CSS holds something it doesn't know at
//! all: an at-rule or a pseudo-class that isn't on its lists ([`Flags`]). The
//! lists are what the pass was written and measured against; a page with
//! anything else is one the pass can't speak for, and leaves alone.

use std::ops::Range;

use cssparser::{Parser, ParserInput, Token};

use super::tree::Element;
use super::{GATE_TOKENS, Gate, MAX_NESTING};
use crate::error::Error;

/// A declaration: its name in lower case, and where its value is in the text
/// it came from, `!important` included.
#[derive(Debug)]
pub(super) struct Decl {
    pub(super) name: String,
    pub(super) value: Range<usize>,
    /// It is in an at-rule's block, inside its rule or around it: it may not
    /// apply when the rule does.
    pub(super) conditional: bool,
}

/// The at-rules this reader knows: what each holds, and that none of them
/// brings in CSS from elsewhere or writes declarations into a rule. `@import`
/// is known and has a flag of its own.
const KNOWN_AT_RULES: &[&str] = &[
    "charset",
    "container",
    "counter-style",
    "font-face",
    "font-feature-values",
    "font-palette-values",
    "import",
    "keyframes",
    "-webkit-keyframes",
    "layer",
    "media",
    "namespace",
    "page",
    "property",
    "scope",
    "starting-style",
    "supports",
    // In `@page`.
    "top-left-corner",
    "top-left",
    "top-center",
    "top-right",
    "top-right-corner",
    "bottom-left-corner",
    "bottom-left",
    "bottom-center",
    "bottom-right",
    "bottom-right-corner",
    "left-top",
    "left-middle",
    "left-bottom",
    "right-top",
    "right-middle",
    "right-bottom",
];

/// The pseudo-classes this reader knows. Each matches an element by its own
/// state or attributes, by its ancestors, or by its place among its siblings:
/// none looks at what an element holds, as `:has()` does, so nothing put into
/// `<head>` changes what any of them matches outside it.
const KNOWN_PSEUDO_CLASSES: &[&str] = &[
    "active",
    "any-link",
    "checked",
    "default",
    "defined",
    "dir",
    "disabled",
    "empty",
    "enabled",
    "first",
    "first-child",
    "first-of-type",
    "focus",
    "focus-visible",
    "focus-within",
    "fullscreen",
    "hover",
    "in-range",
    "indeterminate",
    "invalid",
    "is",
    "lang",
    "last-child",
    "last-of-type",
    "left",
    "link",
    "not",
    "nth-child",
    "nth-last-child",
    "nth-last-of-type",
    "nth-of-type",
    "only-child",
    "only-of-type",
    "open",
    "optional",
    "out-of-range",
    "placeholder-shown",
    "read-only",
    "read-write",
    "required",
    "right",
    "root",
    "scope",
    "target",
    "valid",
    "visited",
    "where",
    "-webkit-any",
    "-webkit-any-link",
    "-webkit-autofill",
    "-moz-any",
    "-moz-any-link",
    "-moz-focusring",
    "-moz-placeholder",
    "-ms-input-placeholder",
];

/// Pseudo-elements that put nothing after an element's content.
const LEADING_PSEUDO_ELEMENTS: &[&str] = &[
    "before",
    "first-line",
    "first-letter",
    "marker",
    "selection",
    "placeholder",
    "backdrop",
    "file-selector-button",
];

/// What the last compound of one selector asks of an element.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Subject {
    /// Nothing is known: it may match any element, and any pseudo-element.
    pub(super) any: bool,
    /// The type selector, in lower case.
    pub(super) tag: Option<String>,
    pub(super) classes: Vec<String>,
    pub(super) id: Option<String>,
    /// It selects a pseudo-element, not the element.
    pub(super) pseudo_element: bool,
    /// That pseudo-element is `::after`, or one this reader doesn't know:
    /// it may put a box after the element's content.
    pub(super) after: bool,
}

impl Subject {
    fn anything() -> Self {
        Self {
            any: true,
            pseudo_element: true,
            after: true,
            ..Self::default()
        }
    }

    /// Whether the selector could match `element`, or a pseudo-element of it.
    /// Names compare without case: more matches than Chrome finds, in quirks
    /// mode exactly as many.
    pub(super) fn may_match(&self, element: &Element) -> bool {
        if self.any {
            return true;
        }
        if let Some(tag) = &self.tag
            && !element.local().eq_ignore_ascii_case(tag)
        {
            return false;
        }
        if let Some(id) = &self.id
            && !element
                .attr("id")
                .is_some_and(|i| i.eq_ignore_ascii_case(id))
        {
            return false;
        }
        let class = element.attr("class").unwrap_or("");
        self.classes.iter().all(|wanted| {
            class
                .split_ascii_whitespace()
                .any(|c| c.eq_ignore_ascii_case(wanted))
        })
    }
}

/// Where a rule is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    /// A style sheet: the `<style>` at this node.
    Sheet(usize),
    /// The `style` attribute of the element at this node.
    Attribute(usize),
}

/// A style rule, or a `style` attribute, which acts as one.
#[derive(Debug)]
pub(super) struct Rule {
    pub(super) origin: Origin,
    /// Which of [`Rules::texts`] it is in.
    pub(super) text: usize,
    /// Its selector's place in that text. Empty for an attribute.
    pub(super) prelude: Range<usize>,
    /// Directly in a sheet: in no at-rule and no other rule.
    pub(super) plain: bool,
    /// One per selector in its list. For an attribute, none: it is its element's.
    pub(super) subjects: Vec<Subject>,
    pub(super) decls: Vec<Decl>,
}

impl Rule {
    /// Whether it could apply to `element` at node `id`.
    pub(super) fn may_match(&self, id: usize, element: &Element) -> bool {
        match self.origin {
            Origin::Attribute(node) => node == id,
            Origin::Sheet(_) => self.subjects.iter().any(|s| s.may_match(element)),
        }
    }
}

/// Words that change what the pass may assume, wherever they are in the CSS.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Flags {
    /// `@import`: CSS this reader can't see.
    pub(super) import: bool,
    /// `:has(`: a selector that looks at what follows or is inside.
    pub(super) has: bool,
    /// `revert-layer`: a value that asks what a lower layer said.
    pub(super) revert_layer: bool,
    /// An at-rule that isn't in [`KNOWN_AT_RULES`].
    pub(super) unknown_at_rule: bool,
    /// A pseudo-class in a selector that isn't in [`KNOWN_PSEUDO_CLASSES`].
    pub(super) unknown_pseudo_class: bool,
}

/// Every rule of a page.
#[derive(Debug, Default)]
pub(super) struct Rules {
    pub(super) texts: Vec<String>,
    pub(super) rules: Vec<Rule>,
    pub(super) flags: Flags,
    /// The CSS nests deeper than a printer may be given, so it wasn't read to
    /// the end: nothing can be said about it.
    pub(super) unread: bool,
    tokens: usize,
}

impl Rules {
    /// Reads the text of the `<style>` at `node`.
    pub(super) fn read_sheet(
        &mut self,
        node: usize,
        css: String,
        gate: &Gate<'_>,
    ) -> Result<(), Error> {
        self.read(Origin::Sheet(node), css, gate)
    }

    /// Reads the `style` attribute of the element at `node`.
    pub(super) fn read_attribute(
        &mut self,
        node: usize,
        css: String,
        gate: &Gate<'_>,
    ) -> Result<(), Error> {
        self.read(Origin::Attribute(node), css, gate)
    }

    fn read(&mut self, origin: Origin, css: String, gate: &Gate<'_>) -> Result<(), Error> {
        let text = self.texts.len();
        let mut found = Found::default();
        let halted = {
            let mut walk = Walk {
                found: &mut found,
                flags: &mut self.flags,
                tokens: &mut self.tokens,
                unread: &mut self.unread,
                gate,
                halted: None,
                css: &css,
            };
            let mut input = ParserInput::new(&css);
            let mut parser = Parser::new(&mut input);
            let top = match origin {
                Origin::Sheet(_) => Context::default(),
                Origin::Attribute(_) => Context {
                    owner: Some(walk.found.open(0..0, Vec::new(), false)),
                    ..Context::default()
                },
            };
            walk.block(&mut parser, top, 0);
            walk.halted
        };
        if let Some(e) = halted {
            return Err(e);
        }
        self.rules.extend(found.rules.into_iter().map(|r| Rule {
            origin,
            text,
            prelude: r.prelude,
            plain: r.plain,
            subjects: r.subjects,
            decls: r.decls,
        }));
        self.texts.push(css);
        Ok(())
    }

    /// The text of `decl`'s value, a declaration of `rule`.
    pub(super) fn value(&self, rule: &Rule, decl: &Decl) -> &str {
        &self.texts[rule.text][decl.value.clone()]
    }

    /// The text of `rule`'s selector.
    pub(super) fn selector(&self, rule: &Rule) -> &str {
        &self.texts[rule.text][rule.prelude.clone()]
    }

    /// How many tokens were read: what reading cost.
    pub(super) fn tokens(&self) -> usize {
        self.tokens
    }
}

/// A rule while its text is read.
struct Open {
    prelude: Range<usize>,
    plain: bool,
    subjects: Vec<Subject>,
    decls: Vec<Decl>,
}

#[derive(Default)]
struct Found {
    rules: Vec<Open>,
}

impl Found {
    fn open(&mut self, prelude: Range<usize>, subjects: Vec<Subject>, plain: bool) -> usize {
        self.rules.push(Open {
            prelude,
            plain,
            subjects,
            decls: Vec::new(),
        });
        self.rules.len() - 1
    }
}

/// Where in a sheet a block is.
#[derive(Debug, Default, Clone, Copy)]
struct Context {
    /// The style rule declarations here belong to.
    owner: Option<usize>,
    /// In an at-rule's block.
    in_at: bool,
    /// Rules here aren't matched by their selectors: keyframes, and the blocks
    /// of at-rules this reader doesn't know.
    loose: bool,
}

/// What a block's content has shown of the item being read: a declaration, or
/// a rule's prelude.
#[derive(Default)]
struct Item {
    /// Where its first token starts.
    start: Option<usize>,
    /// Its first token, if that's a name: an identifier's, in lower case.
    ident: Option<String>,
    /// Its first token, if that's an at-keyword, in lower case.
    at: Option<String>,
    /// A colon came second, and the value starts here.
    value: Option<usize>,
    count: usize,
}

impl Item {
    fn note(&mut self, start: usize) {
        self.count += 1;
        self.start.get_or_insert(start);
    }

    /// A custom property's declaration, which a `{}` doesn't end.
    fn custom(&self) -> bool {
        self.value.is_some() && self.ident.as_deref().is_some_and(|n| n.starts_with("--"))
    }
}

struct Walk<'w, 'g> {
    found: &'w mut Found,
    flags: &'w mut Flags,
    tokens: &'w mut usize,
    unread: &'w mut bool,
    gate: &'w Gate<'g>,
    halted: Option<Error>,
    css: &'w str,
}

impl Walk<'_, '_> {
    /// Counts a token, and says whether reading has to stop.
    fn stop(&mut self) -> bool {
        *self.tokens += 1;
        if self.tokens.is_multiple_of(GATE_TOKENS) && self.halted.is_none() {
            self.halted = self.gate.check().err();
        }
        self.halted.is_some() || *self.unread
    }

    fn word(&mut self, name: &str) {
        self.flags.revert_layer |= name.eq_ignore_ascii_case("revert-layer");
    }

    fn at_keyword(&mut self, name: &str) {
        let name = name.to_ascii_lowercase();
        self.flags.import |= name == "import";
        self.flags.unknown_at_rule |= !KNOWN_AT_RULES.contains(&name.as_str());
    }

    /// Looks at the pseudo-classes of the selectors at `prelude`, at any depth.
    fn pseudo_classes(&mut self, prelude: Range<usize>) {
        let css = self.css;
        let mut input = ParserInput::new(&css[prelude]);
        let mut p = Parser::new(&mut input);
        self.pseudo_classes_in(&mut p, 0);
    }

    fn pseudo_classes_in(&mut self, p: &mut Parser<'_, '_>, depth: usize) {
        // Colons just before this token: one for a pseudo-class.
        let mut colons = 0;
        while let Ok(token) = p.next_including_whitespace_and_comments() {
            let token = token.clone();
            if self.stop() {
                return;
            }
            match &token {
                Token::Colon => {
                    colons += 1;
                    continue;
                }
                Token::Ident(name) | Token::Function(name) if colons == 1 => {
                    let name = name.to_ascii_lowercase();
                    let known =
                        KNOWN_PSEUDO_CLASSES.contains(&name.as_str()) || old_pseudo_element(&name);
                    self.flags.unknown_pseudo_class |= !known;
                }
                _ => {}
            }
            colons = 0;
            if opens(&token) {
                self.nested(p, depth, |w, p| w.pseudo_classes_in(p, depth + 1));
            }
        }
    }

    fn function(&mut self, name: &str) {
        self.flags.has |= name.eq_ignore_ascii_case("has");
    }

    fn nested(
        &mut self,
        p: &mut Parser<'_, '_>,
        depth: usize,
        inner: impl FnOnce(&mut Self, &mut Parser<'_, '_>),
    ) {
        if depth >= MAX_NESTING {
            *self.unread = true;
            return;
        }
        let read: Result<(), cssparser::ParseError<'_, ()>> = p.parse_nested_block(|p| {
            inner(self, p);
            Ok(())
        });
        drop(read);
    }

    /// What's in a bracket or a function: only its words count.
    fn scan(&mut self, p: &mut Parser<'_, '_>, depth: usize) {
        while let Ok(token) = p.next_including_whitespace_and_comments() {
            let token = token.clone();
            if self.stop() {
                return;
            }
            match &token {
                Token::Ident(name) => self.word(name),
                Token::AtKeyword(name) => self.at_keyword(name),
                Token::Function(name) => self.function(name),
                _ => {}
            }
            if opens(&token) {
                self.nested(p, depth, |w, p| w.scan(p, depth + 1));
            }
        }
    }

    /// A block's content, or a sheet's: declarations and rules, told apart as
    /// CSS Syntax does. What ends at a `;` is a declaration if it starts with
    /// a name and a colon. What ends at a `{}` is a rule, unless it is a custom
    /// property's value.
    fn block(&mut self, p: &mut Parser<'_, '_>, context: Context, depth: usize) {
        let mut item = Item::default();
        loop {
            let before = p.position().byte_index();
            let Ok(token) = p.next_including_whitespace_and_comments() else {
                break;
            };
            let token = token.clone();
            if self.stop() {
                return;
            }
            let after = p.position().byte_index();
            match &token {
                Token::WhiteSpace(_) | Token::Comment(_) => continue,
                Token::Semicolon => {
                    self.declaration(&item, before, context);
                    item = Item::default();
                    continue;
                }
                Token::CurlyBracketBlock if !item.custom() => {
                    let prelude = item.start.unwrap_or(before)..before;
                    let inner = match &item.at {
                        Some(name) => {
                            if name == "scope" {
                                self.pseudo_classes(prelude);
                            }
                            self.at_rule(name, context)
                        }
                        None => {
                            self.pseudo_classes(prelude.clone());
                            self.rule(prelude, context)
                        }
                    };
                    self.nested(p, depth, |w, p| w.block(p, inner, depth + 1));
                    item = Item::default();
                    continue;
                }
                Token::Ident(name) => {
                    self.word(name);
                    if item.count == 0 {
                        item.ident = Some(name.to_ascii_lowercase());
                    }
                }
                Token::AtKeyword(name) => {
                    self.at_keyword(name);
                    if item.count == 0 {
                        item.at = Some(name.to_ascii_lowercase());
                    }
                }
                Token::Colon if item.count == 1 && item.ident.is_some() => {
                    item.value = Some(after);
                }
                Token::Function(name) => self.function(name),
                _ => {}
            }
            item.note(before);
            if opens(&token) {
                self.nested(p, depth, |w, p| w.scan(p, depth + 1));
            }
        }
        self.declaration(
            &item,
            self.css.len().min(p.position().byte_index()),
            context,
        );
    }

    fn declaration(&mut self, item: &Item, end: usize, context: Context) {
        let (Some(name), Some(start), Some(owner)) = (&item.ident, item.value, context.owner)
        else {
            // With no rule around it, it is an at-rule's descriptor, and no
            // element's property.
            return;
        };
        self.found.rules[owner].decls.push(Decl {
            name: name.clone(),
            value: start..end.max(start),
            conditional: context.in_at,
        });
    }

    /// The context inside the block of the at-rule `name`.
    fn at_rule(&mut self, name: &str, context: Context) -> Context {
        let groups = matches!(
            name,
            "media" | "supports" | "layer" | "container" | "starting-style" | "document"
        );
        if name == "scope" {
            // Declarations straight in `@scope` are its roots', whichever
            // elements those are.
            let owner = self.found.open(0..0, vec![Subject::anything()], false);
            return Context {
                owner: Some(owner),
                in_at: true,
                loose: true,
            };
        }
        Context {
            owner: context.owner,
            in_at: true,
            loose: context.loose || !groups,
        }
    }

    /// Opens the rule whose selector is at `prelude`, and gives the context
    /// inside its block.
    fn rule(&mut self, prelude: Range<usize>, context: Context) -> Context {
        let nested = context.owner.is_some();
        let subjects = if nested || context.loose {
            vec![Subject::anything()]
        } else {
            self.subjects(prelude.clone())
        };
        let plain = !nested && !context.in_at;
        let owner = self.found.open(prelude, subjects, plain);
        Context {
            owner: Some(owner),
            ..context
        }
    }

    /// One [`Subject`] per selector in the list at `prelude`.
    fn subjects(&mut self, prelude: Range<usize>) -> Vec<Subject> {
        let text = &self.css[prelude];
        let mut input = ParserInput::new(text);
        let mut p = Parser::new(&mut input);
        let mut all = Vec::new();
        // The tokens of the compound being read, brackets as one token each.
        let mut compound: Vec<Token<'_>> = Vec::new();
        let mut broken = false;
        let mut gap = false;
        loop {
            let token = p.next_including_whitespace_and_comments().cloned();
            if self.stop() {
                return vec![Subject::anything()];
            }
            match token {
                Err(_) | Ok(Token::Comma) => {
                    all.push(if broken {
                        Subject::anything()
                    } else {
                        subject(&compound)
                    });
                    compound.clear();
                    (broken, gap) = (false, false);
                    if token.is_err() {
                        break;
                    }
                }
                Ok(Token::Comment(_)) => {}
                Ok(Token::WhiteSpace(_)) => gap = !compound.is_empty(),
                Ok(Token::Delim('>' | '+' | '~')) => {
                    compound.clear();
                    gap = false;
                }
                Ok(other) => {
                    if gap {
                        compound.clear();
                    }
                    gap = false;
                    // What's in a bracket only narrows what the compound
                    // matches. A bracket that can't be read leaves it unknown.
                    broken |= matches!(
                        other,
                        Token::BadString(_)
                            | Token::BadUrl(_)
                            | Token::CloseParenthesis
                            | Token::CloseSquareBracket
                            | Token::CloseCurlyBracket
                            | Token::CurlyBracketBlock
                    );
                    compound.push(other);
                }
            }
        }
        all
    }
}

fn opens(token: &Token<'_>) -> bool {
    matches!(
        token,
        Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock
    )
}

/// The four pseudo-elements CSS 2 wrote with one colon.
fn old_pseudo_element(name: &str) -> bool {
    ["before", "after", "first-line", "first-letter"]
        .iter()
        .any(|n| name.eq_ignore_ascii_case(n))
}

/// What a compound selector's tokens ask of an element.
fn subject(compound: &[Token<'_>]) -> Subject {
    let mut s = Subject::default();
    let mut tokens = compound.iter().peekable();
    match tokens.peek() {
        None => return Subject::anything(),
        Some(Token::Ident(tag)) => {
            s.tag = Some(tag.to_ascii_lowercase());
            tokens.next();
        }
        Some(Token::Delim('*')) => {
            tokens.next();
        }
        _ => {}
    }
    while let Some(token) = tokens.next() {
        match token {
            Token::IDHash(id) => s.id = Some(id.to_string()),
            Token::Delim('.') => match tokens.next() {
                Some(Token::Ident(class)) => s.classes.push(class.to_string()),
                _ => return Subject::anything(),
            },
            Token::SquareBracketBlock => {}
            Token::Colon => match tokens.next() {
                Some(Token::Colon) => {
                    let leading = match tokens.next() {
                        Some(Token::Ident(name) | Token::Function(name)) => LEADING_PSEUDO_ELEMENTS
                            .iter()
                            .any(|known| name.eq_ignore_ascii_case(known)),
                        _ => return Subject::anything(),
                    };
                    s.pseudo_element = true;
                    s.after |= !leading;
                }
                Some(Token::Ident(name)) if old_pseudo_element(name) => {
                    s.pseudo_element = true;
                    s.after |= name.eq_ignore_ascii_case("after");
                }
                // `:root` is the `html` element here, and no other.
                Some(Token::Ident(name)) if name.eq_ignore_ascii_case("root") => {
                    if s.tag.as_deref().is_some_and(|t| t != "html") {
                        return Subject::anything();
                    }
                    s.tag = Some("html".to_owned());
                }
                Some(Token::Ident(_) | Token::Function(_)) => {}
                _ => return Subject::anything(),
            },
            // A namespace prefix, a nesting selector, or nothing a selector has.
            _ => return Subject::anything(),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use super::*;

    fn read(css: &str) -> Rules {
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::from_secs(60));
        let mut rules = Rules::default();
        rules
            .read_sheet(1, css.to_owned(), &gate)
            .unwrap_or_else(|e| panic!("{e}"));
        rules
    }

    fn names(rules: &Rules) -> Vec<(String, Vec<String>)> {
        rules
            .rules
            .iter()
            .map(|r| {
                (
                    rules.selector(r).trim().to_owned(),
                    r.decls.iter().map(|d| d.name.clone()).collect(),
                )
            })
            .collect()
    }

    fn pair(selector: &str, decls: &[&str]) -> (String, Vec<String>) {
        (
            selector.to_owned(),
            decls.iter().map(|d| (*d).to_owned()).collect(),
        )
    }

    #[test]
    fn rules_and_declarations_are_told_apart_as_css_syntax_does() {
        let rules = read(
            "@charset \"x\"; a:hover, .b > c { COLOR: red; --x: {a:b}; width : 1px }\n\
             @media print { d { e: f; g:hover { h: i } j: k } }\n\
             l { m: n;; o:p }",
        );
        assert_eq!(
            names(&rules),
            [
                pair("a:hover, .b > c", &["color", "--x", "width"]),
                pair("d", &["e", "j"]),
                pair("g:hover", &["h"]),
                pair("l", &["m", "o"]),
            ]
        );
        let first = &rules.rules[0];
        assert_eq!(rules.value(first, &first.decls[0]).trim(), "red");
        assert_eq!(rules.value(first, &first.decls[1]).trim(), "{a:b}");
        assert_eq!(rules.value(first, &first.decls[2]).trim(), "1px");
        assert!(first.plain && !rules.rules[1].plain && !rules.rules[2].plain);
        assert!(rules.rules[3].plain);
    }

    #[test]
    fn a_descriptor_is_no_elements_property() {
        let rules = read(
            "@font-face { font-family: x; display: block }\n\
             @page { margin: 1in; @top-left { content: 'x' } }\n\
             display: block; p { color: red }",
        );
        assert_eq!(names(&rules), [pair("p", &["color"])]);
    }

    #[test]
    fn what_an_at_rule_holds_in_a_style_rule_is_that_rules() {
        let rules = read("p { @media print { display: none } @supports (x:y) { all: unset } }");
        assert_eq!(names(&rules), [pair("p", &["display", "all"])]);
    }

    fn subjects(css: &str) -> Vec<Subject> {
        let rules = read(css);
        rules
            .rules
            .iter()
            .flat_map(|r| r.subjects.clone())
            .collect()
    }

    fn known(tag: Option<&str>, classes: &[&str], id: Option<&str>) -> Subject {
        Subject {
            tag: tag.map(str::to_owned),
            classes: classes.iter().map(|c| (*c).to_owned()).collect(),
            id: id.map(str::to_owned),
            ..Subject::default()
        }
    }

    #[test]
    fn a_selectors_last_compound_says_what_it_can_match() {
        assert_eq!(
            subjects("div p.a.b#c:hover[x=\"y z\"]:not(.d, e) , ul>LI , *.x , a + b~ c {}"),
            [
                known(Some("p"), &["a", "b"], Some("c")),
                known(Some("li"), &[], None),
                known(None, &["x"], None),
                known(Some("c"), &[], None),
            ]
        );
        assert_eq!(subjects(":root{}"), [known(Some("html"), &[], None)]);
        assert_eq!(subjects(".a\\:b{}"), [known(None, &["a:b"], None)]);
        let after = Subject {
            pseudo_element: true,
            after: true,
            ..known(Some("pre"), &[], None)
        };
        assert_eq!(subjects("pre::after{} pre:AFTER{}"), [after.clone(), after]);
        let before = Subject {
            pseudo_element: true,
            ..known(None, &["q"], None)
        };
        assert_eq!(subjects(".q::before{}"), [before]);
        // One this reader doesn't know may put a box after the content too.
        let unknown = Subject {
            pseudo_element: true,
            after: true,
            ..known(None, &["q"], None)
        };
        assert_eq!(
            subjects(".q::scroll-marker{} .q::-webkit-x{}"),
            [unknown.clone(), unknown]
        );
    }

    #[test]
    fn what_isnt_understood_matches_everything() {
        for css in [
            "svg|a {}",
            "*|a {}",
            ".a& {}",
            "&.a {}",
            "a b% {}",
            "{}",
            "a, {}",
            "p { .nested {} }",
            "@keyframes k { from {} }",
            "@unknown { a {} }",
            "@scope (.x) { color: red }",
            "a:: {}",
            "a b) {}",
        ] {
            let all = subjects(css);
            assert!(all.iter().any(|s| s.any), "{css}: {all:?}");
        }
        assert!(subjects("@media print { a {} }").iter().all(|s| !s.any));
    }

    #[test]
    fn words_are_found_wherever_they_are() {
        let flags = |css: &str| read(css).flags;
        assert!(flags("@import 'x';").import);
        assert!(flags("@media print { @IMPORT url(x); }").import);
        assert!(flags("a:is(b, :has(c)) {}").has);
        assert!(flags("a { color: red; width: REVERT-LAYER }").revert_layer);
        assert!(flags("a { --x: { y: revert-layer } }").revert_layer);
        let none = flags("a:not(.has) { content: \":has( revert-layer @import\" } /* :has( */");
        assert!(!none.import && !none.has && !none.revert_layer);
        assert!(!none.unknown_at_rule && !none.unknown_pseudo_class);
    }

    #[test]
    fn what_the_reader_doesnt_know_is_flagged() {
        let flags = |css: &str| read(css).flags;
        for css in [
            "@apply --mixin;",
            "a { @apply --mixin; }",
            "@mixin --m() { color: red }",
            "@function --f() { result: 1 }",
            "@media print { @whatever { a {} } }",
            "@position-try --x { top: 0 }",
        ] {
            assert!(flags(css).unknown_at_rule, "{css}");
        }
        for css in [
            "@charset 'x'; @namespace s url(x); @layer a, b; @media print { a {} }",
            "@page :first { margin: 0; @top-center { content: 'x' } }",
            "@font-face { font-family: x } @keyframes k { from {} } @-webkit-keyframes k { to {} }",
            "@supports (display: grid) { a {} } @container (min-width: 1px) { a {} }",
            "@property --x { syntax: '*'; inherits: false } @scope (.a) to (.b) { c {} }",
        ] {
            assert!(!flags(css).unknown_at_rule, "{css}");
        }
        for css in [
            "a:has-slotted {}",
            "a:host(.b) c {}",
            "a:is(b, :nth-child(2 of :state(x))) {}",
            "a:not(:unknown) {}",
            "@scope (a:popover-open) { b {} }",
            "a { &:future-thing {} }",
            "@media print { a:target-within {} }",
        ] {
            assert!(flags(css).unknown_pseudo_class, "{css}");
        }
        for css in [
            "a:hover, b:not(.c):nth-child(2n + 1)::after, d:is(e, f:FIRST-CHILD) {}",
            "a::-webkit-scrollbar, b::unknown-element, c:before, d::part(x) {}",
            "a { color: red; background: url(x:y) } @media (orientation: portrait) { a {} }",
            "a:-webkit-any-link, input:-moz-placeholder, :root, :lang(ja), :where(:focus-within) {}",
            "a { b: c:d; } [x|y=\"a:b\"] {}",
        ] {
            assert!(!flags(css).unknown_pseudo_class, "{css}");
        }
    }

    #[test]
    fn a_declaration_knows_whether_it_is_conditional() {
        let rules = read("a { b: c; @media print { d: e } f: g } @media print { h { i: j } }");
        let all: Vec<_> = rules
            .rules
            .iter()
            .flat_map(|r| r.decls.iter().map(|d| (d.name.as_str(), d.conditional)))
            .collect();
        assert_eq!(all, [("b", false), ("d", true), ("f", false), ("i", true)]);
    }

    #[test]
    fn css_nested_too_deeply_is_left_unread() {
        let deep = format!("a {{ b: {}x{} }}", "(".repeat(40), ")".repeat(40));
        assert!(read(&deep).unread);
        assert!(!read("a { b: ((x)) }").unread);
    }

    #[test]
    fn an_attribute_is_one_rule_of_its_element() {
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::from_secs(60));
        let mut rules = Rules::default();
        rules
            .read_attribute(7, "color: red; DISPLAY: none".to_owned(), &gate)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(rules.rules.len(), 1);
        assert_eq!(rules.rules[0].origin, Origin::Attribute(7));
        let names: Vec<_> = rules.rules[0].decls.iter().map(|d| &d.name).collect();
        assert_eq!(names, ["color", "display"]);
    }
}
