//! The shape of style text that layout will take, checked on tokens before
//! any of it is given to the style engine.
//!
//! A layout's style pass is one call that can't be stopped once started, so
//! what goes into it is bounded here first, the way a `style` attribute is
//! bounded when it is written ([`crate::style_attr::fits`]) and the way the
//! facade's printer screen bounds a page's CSS before it is printed
//! (`crates/surimono/src/print.rs`). **The numbers here and there must
//! agree**: a page the printer takes should be one layout takes, and the
//! other way round. This crate can't depend on the facade; one shared screen
//! is a follow-up.
//!
//! The pass reads each token once, nests as deep as [`MAX_NESTING`] and no
//! deeper, and looks at the deadline as it goes.

use std::collections::HashMap;

use cssparser::{ParseError, Parser, ParserInput, Token};
use surimono_core::Deadline;

/// Deepest nesting of `{}`, `()`, `[]` and functions. As `MAX_NESTING` in the
/// printer screen and in `style_attr`.
pub(crate) const MAX_NESTING: usize = 32;
/// Most declarations in one block. As `MAX_BLOCK_DECLARATIONS` there.
pub(crate) const MAX_BLOCK_DECLARATIONS: u64 = 4_096;
/// What a page's blocks may cost, each charged its declarations times
/// themselves, an `all` as [`ALL_LONGHANDS`] more. As `MAX_STEPS` there.
pub(crate) const MAX_STEPS: u64 = 1 << 26;
/// Longhands one `all` stands for, rounded up.
const ALL_LONGHANDS: u64 = 512;
/// Most words in the strings of one grid or custom-property declaration.
pub(crate) const MAX_CELLS: usize = 4_096;
/// Most such words in a page.
pub(crate) const MAX_PAGE_CELLS: usize = 65_536;
/// Longest chain of custom properties naming each other through `var()`.
pub(crate) const MAX_VAR_CHAIN: usize = 64;
/// Most combinators in one selector.
pub(crate) const MAX_COMBINATORS: usize = 64;
/// Tokens between looks at the deadline.
const POLL_TOKENS: usize = 4_096;

/// Why a screen ended early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// Over one of the bounds: layout isn't run for this page.
    TooMuch,
    /// The script's deadline passed.
    TimeLimit,
}

/// One `{}` block's count.
#[derive(Default)]
struct Block {
    declarations: u64,
    alls: u64,
    /// At the start of a declaration, where its name is.
    at_name: bool,
    /// The declaration's name is `grid…` or a custom property's.
    counts_cells: bool,
    cells: usize,
    /// The custom property being declared, if one is.
    from: Option<usize>,
}

/// What a page's style text has used of the bounds, over all its sheets and
/// style attributes.
#[derive(Default)]
pub(crate) struct Screen {
    steps: u64,
    cells: usize,
    tokens: usize,
    refused: Option<Refused>,
    /// Custom properties by name. Only looked up.
    ids: HashMap<String, usize>,
    /// By id: the ids its declarations name, with repeats.
    names: Vec<Vec<usize>>,
}

type Failure<'i> = ParseError<'i, ()>;

impl Screen {
    fn refuse(&mut self, why: Refused) {
        self.refused.get_or_insert(why);
    }

    fn id(&mut self, name: &str) -> usize {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = self.names.len();
        self.ids.insert(name.to_owned(), id);
        self.names.push(Vec::new());
        id
    }

    /// Reads one sheet's text.
    pub(crate) fn sheet(&mut self, css: &str, deadline: Option<&Deadline>) {
        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        let mut top = Block::default();
        self.walk(&mut parser, &mut top, 0, deadline);
    }

    /// Notes what a custom property declared in a style attribute names. The
    /// attribute's own shape was bounded when it was written.
    pub(crate) fn attribute_declaration(&mut self, name: &str, value: &str) {
        if !name.starts_with("--") {
            return;
        }
        let from = self.id(name);
        let mut input = ParserInput::new(value);
        let mut parser = Parser::new(&mut input);
        let mut block = Block {
            from: Some(from),
            ..Block::default()
        };
        self.walk(&mut parser, &mut block, 1, None);
    }

    /// Whether everything read fits, the chains of custom properties included.
    pub(crate) fn finish(mut self) -> Result<(), Refused> {
        if self.refused.is_none() && self.longest_chain() > MAX_VAR_CHAIN {
            self.refuse(Refused::TooMuch);
        }
        self.refused.map_or(Ok(()), Err)
    }

    /// Reads tokens to the end of this parser's block and answers the most
    /// combinators in any comma-separated entry of what it read since the
    /// last `;` or `{}`: a selector's, if a `{}` comes next.
    fn walk(
        &mut self,
        p: &mut Parser<'_, '_>,
        block: &mut Block,
        depth: usize,
        deadline: Option<&Deadline>,
    ) -> usize {
        // Combinators in the entry being read and the most in one before it;
        // whether a compound selector has started and a space followed it.
        let (mut most, mut here) = (0usize, 0usize);
        let (mut started, mut gap) = (false, false);
        while self.refused.is_none() {
            let Ok(token) = p.next_including_whitespace_and_comments() else {
                break;
            };
            self.tokens += 1;
            if self.tokens.is_multiple_of(POLL_TOKENS) && deadline.is_some_and(Deadline::passed) {
                self.refuse(Refused::TimeLimit);
            }
            let token = token.clone();
            match &token {
                Token::Comment(_) => continue,
                Token::WhiteSpace(_) => {
                    gap = started;
                    continue;
                }
                Token::Semicolon => {
                    block.declarations += 1;
                    block.at_name = true;
                    block.counts_cells = false;
                    block.cells = 0;
                    block.from = None;
                    (most, here, started, gap) = (0, 0, false, false);
                    continue;
                }
                Token::Comma => {
                    most = most.max(here);
                    (here, started, gap) = (0, false, false);
                    continue;
                }
                Token::Delim('>' | '+' | '~') => {
                    here += 1;
                    (started, gap) = (false, false);
                    block.at_name = false;
                    continue;
                }
                Token::Ident(name) if block.at_name => {
                    block.at_name = false;
                    block.alls += u64::from(name.eq_ignore_ascii_case("all"));
                    let custom = name.starts_with("--");
                    block.counts_cells = custom
                        || name
                            .get(..4)
                            .is_some_and(|n| n.eq_ignore_ascii_case("grid"));
                    block.from = custom.then(|| self.id(name));
                }
                Token::QuotedString(s) if block.counts_cells => {
                    let cells = s.split_ascii_whitespace().count();
                    block.cells += cells;
                    self.cells += cells;
                    if block.cells > MAX_CELLS || self.cells > MAX_PAGE_CELLS {
                        self.refuse(Refused::TooMuch);
                    }
                }
                _ => {}
            }
            block.at_name = false;
            // A space before a block ends the selector; it joins nothing.
            if !matches!(token, Token::CurlyBracketBlock) {
                here += usize::from(gap);
            }
            (started, gap) = (true, false);
            let is_var = matches!(&token, Token::Function(f) if f.eq_ignore_ascii_case("var"));
            match token {
                Token::CurlyBracketBlock => {
                    if most.max(here) > MAX_COMBINATORS {
                        self.refuse(Refused::TooMuch);
                    }
                    let mut inner = Block {
                        at_name: true,
                        ..Block::default()
                    };
                    self.nested(p, &mut inner, depth, false, deadline);
                    self.close(&inner);
                    block.at_name = true;
                    (most, here, started, gap) = (0, 0, false, false);
                }
                Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                    // What a pseudo-class holds counts for the entry it is in.
                    here += self.nested(p, block, depth, is_var, deadline);
                    // Whatever was in the brackets, a value goes on after them.
                    block.at_name = false;
                }
                _ => {}
            }
        }
        most.max(here)
    }

    fn nested(
        &mut self,
        p: &mut Parser<'_, '_>,
        block: &mut Block,
        depth: usize,
        var: bool,
        deadline: Option<&Deadline>,
    ) -> usize {
        if depth >= MAX_NESTING {
            self.refuse(Refused::TooMuch);
            return 0;
        }
        let mut combinators = 0;
        let result: Result<(), Failure<'_>> = p.parse_nested_block(|p| {
            if var && let Some(from) = block.from {
                let start = p.state();
                if let Ok(Token::Ident(name)) = p.next() {
                    let name = name.clone();
                    let to = self.id(&name);
                    self.names[from].push(to);
                }
                p.reset(&start);
            }
            combinators = self.walk(p, block, depth + 1, deadline);
            Ok(())
        });
        drop(result);
        combinators
    }

    fn close(&mut self, block: &Block) {
        let declarations = block.declarations + 1;
        if declarations > MAX_BLOCK_DECLARATIONS {
            self.refuse(Refused::TooMuch);
        }
        let cost = declarations
            .saturating_mul(declarations.saturating_add(block.alls.saturating_mul(ALL_LONGHANDS)));
        self.steps = self.steps.saturating_add(cost);
        if self.steps > MAX_STEPS {
            self.refuse(Refused::TooMuch);
        }
    }

    /// The most links on any path between custom properties. Properties
    /// naming each other in a cycle are one component, as long as all of
    /// them. Tarjan's algorithm without recursion, as the printer screen has
    /// it (`print/chains.rs`).
    fn longest_chain(&self) -> usize {
        const UNSEEN: usize = usize::MAX;
        let count = self.names.len();
        let mut order = vec![UNSEEN; count];
        let mut low = vec![0; count];
        let mut component = vec![UNSEEN; count];
        // Properties on each component's longest path, by component.
        let mut lengths: Vec<usize> = Vec::new();
        let mut open = Vec::new();
        let mut next = 0;
        for root in 0..count {
            if order[root] != UNSEEN {
                continue;
            }
            order[root] = next;
            low[root] = next;
            next += 1;
            open.push(root);
            let mut calls = vec![(root, 0)];
            while let Some(top) = calls.last_mut() {
                let (id, at) = *top;
                if let Some(&to) = self.names[id].get(at) {
                    top.1 += 1;
                    if order[to] == UNSEEN {
                        order[to] = next;
                        low[to] = next;
                        next += 1;
                        open.push(to);
                        calls.push((to, 0));
                    } else if component[to] == UNSEEN {
                        low[id] = low[id].min(order[to]);
                    }
                    continue;
                }
                calls.pop();
                if let Some(&(parent, _)) = calls.last() {
                    low[parent] = low[parent].min(low[id]);
                }
                if low[id] != order[id] {
                    continue;
                }
                let this = lengths.len();
                let mut members = Vec::new();
                while let Some(member) = open.pop() {
                    component[member] = this;
                    members.push(member);
                    if member == id {
                        break;
                    }
                }
                let onward = members
                    .iter()
                    .flat_map(|&member| &self.names[member])
                    .filter(|&&to| component[to] != this)
                    .map(|&to| lengths[component[to]])
                    .max()
                    .unwrap_or(0);
                lengths.push(members.len() + onward);
            }
        }
        lengths.iter().max().map_or(0, |longest| longest - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fits(css: &str) -> bool {
        let mut screen = Screen::default();
        screen.sheet(css, None);
        screen.finish().is_ok()
    }

    #[test]
    fn ordinary_sheets_fit() {
        assert!(fits(""));
        assert!(fits(
            "@media (min-width: 10px) and (max-width: 90px) { a > b + c ~ d e, f:not(.g, .h i) { color: red; margin: 0 auto } }"
        ));
        assert!(fits(
            ":root { --a: 1px; --b: var(--a); --c: calc(var(--b) * 2) } p { width: var(--c, var(--a)) }"
        ));
        // A long list of short selectors, and a value with many spaces.
        assert!(fits(&format!(
            "{}a {{ font: {}x }}",
            "a b, ".repeat(MAX_COMBINATORS * 2),
            "1px ".repeat(200)
        )));
    }

    #[test]
    fn nesting_is_bounded_for_every_kind_of_bracket() {
        for (open, close) in [("a{", "}"), ("(", ")"), ("[", "]"), (":is(", ")")] {
            let nested = |n: usize| format!("{}b{}{{}}", open.repeat(n), close.repeat(n));
            assert!(fits(&nested(MAX_NESTING)), "{open} at the bound");
            assert!(!fits(&nested(MAX_NESTING + 1)), "{open} over the bound");
        }
    }

    #[test]
    fn declarations_in_a_block_are_bounded() {
        let block = |n: usize| format!("a {{ {} }}", "--x:1;".repeat(n));
        assert!(fits(&block(MAX_BLOCK_DECLARATIONS as usize - 1)));
        assert!(!fits(&block(MAX_BLOCK_DECLARATIONS as usize)));
        // Many blocks under the bound each still add up.
        let many = block(4_000).repeat(5);
        assert!(!fits(&many));
    }

    #[test]
    fn combinators_in_one_selector_are_bounded() {
        for with in [" b", ">b", " + b", "~ b"] {
            let rule = |n: usize| format!("a{} {{}}", with.repeat(n));
            assert!(fits(&rule(MAX_COMBINATORS)), "{with:?} at the bound");
            assert!(!fits(&rule(MAX_COMBINATORS + 1)), "{with:?} over the bound");
        }
        // Inside a pseudo-class they count for the selector it is in.
        let inner = format!("a:is(b{}) {{}}", " c".repeat(MAX_COMBINATORS + 1));
        assert!(!fits(&inner));
    }

    #[test]
    fn chains_of_custom_properties_are_bounded() {
        let chain = |n: usize| {
            let links: String = (0..n)
                .map(|i| format!("--p{i}: var(--p{});", i + 1))
                .collect();
            format!("a {{ {links} }}")
        };
        assert!(fits(&chain(MAX_VAR_CHAIN)));
        assert!(!fits(&chain(MAX_VAR_CHAIN + 1)));
        // Split over sheets and style attributes, it is still one chain.
        let mut screen = Screen::default();
        for i in 0..=MAX_VAR_CHAIN {
            if i % 2 == 0 {
                screen.sheet(&format!("a {{ --p{i}: var(--p{}) }}", i + 1), None);
            } else {
                screen.attribute_declaration(&format!("--p{i}"), &format!("var(--p{})", i + 1));
            }
        }
        assert_eq!(screen.finish(), Err(Refused::TooMuch));
    }

    #[test]
    fn grid_words_are_bounded() {
        let grid = |n: usize| format!("a {{ grid-template-areas: '{}' }}", "a ".repeat(n));
        assert!(fits(&grid(MAX_CELLS)));
        assert!(!fits(&grid(MAX_CELLS + 1)));
        // Other strings aren't grids.
        assert!(fits(&format!(
            "a {{ content: '{}' }}",
            "a ".repeat(MAX_CELLS + 1)
        )));
    }
}
