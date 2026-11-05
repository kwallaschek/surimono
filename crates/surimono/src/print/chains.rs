//! Custom properties that name each other, and how long selectors are, as the
//! CSS parser's own rules and declarations have them. Running after the token
//! screen, which has already bounded nesting, and under the same gate. Each
//! level is read once: a declaration gives up at a `{` it can't hold.

use std::collections::HashMap;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};

use super::{GATE_TOKENS, Gate};
use crate::error::Error;

/// Which custom properties name which, and the longest selector, over a page.
#[derive(Debug, Default)]
pub(super) struct References {
    ids: HashMap<String, usize>,
    /// By id: the ids its declarations name, with repeats.
    names: Vec<Vec<usize>>,
    /// The most combinators in one selector.
    pub(super) combinators: usize,
    /// Tokens read so far.
    pub(super) tokens: usize,
    /// The gate said stop.
    halted: Option<Error>,
}

impl References {
    fn id(&mut self, name: &str) -> usize {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = self.names.len();
        self.ids.insert(name.to_owned(), id);
        self.names.push(Vec::new());
        id
    }

    /// Reads a stylesheet.
    pub(super) fn read_sheet(&mut self, css: &str, gate: &Gate<'_>) -> Result<(), Error> {
        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        let mut reader = Reader {
            references: self,
            gate: *gate,
            declarations: false,
            rules: true,
        };
        let mut items = StyleSheetParser::new(&mut parser, &mut reader);
        while items.next().is_some() && items.parser.references.halted.is_none() {}
        self.finish()
    }

    /// Reads a `style` attribute.
    pub(super) fn read_attribute(&mut self, css: &str, gate: &Gate<'_>) -> Result<(), Error> {
        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        let mut reader = Reader {
            references: self,
            gate: *gate,
            declarations: true,
            rules: false,
        };
        let mut items = RuleBodyParser::new(&mut parser, &mut reader);
        while items.next().is_some() && items.parser.references.halted.is_none() {}
        self.finish()
    }

    fn finish(&mut self) -> Result<(), Error> {
        self.halted.take().map_or(Ok(()), Err)
    }

    /// Counts a token, and says whether reading has to stop.
    fn stop(&mut self, gate: &Gate<'_>) -> bool {
        self.tokens += 1;
        if self.tokens.is_multiple_of(GATE_TOKENS) && self.halted.is_none() {
            self.halted = gate.check().err();
        }
        self.halted.is_some()
    }

    /// The most links on any path. Properties naming each other in a cycle are
    /// one component, as long as all of them, so a loop off to the side doesn't
    /// count against what merely uses it. Tarjan's algorithm, without recursion:
    /// a component is done only after those it names.
    pub(super) fn longest_chain(&self) -> usize {
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

type Failure<'i> = ParseError<'i, ()>;

/// Goes through a list of rules or declarations, noting what's in them.
struct Reader<'r, 'g> {
    references: &'r mut References,
    gate: Gate<'g>,
    /// Declarations can be here, as in a style rule or an attribute.
    declarations: bool,
    rules: bool,
}

impl Reader<'_, '_> {
    /// Whether a `{}` is in what's left of `input`, at its own level. `from` is
    /// the custom property the value belongs to, if it does. Any other value
    /// ends at the first `{}`, which makes it a rule, read once as that.
    fn read_value<'i>(&mut self, input: &mut Parser<'i, '_>, from: Option<usize>) -> bool {
        let mut block = false;
        while let Ok(token) = input.next_including_whitespace_and_comments() {
            let reference = match token {
                Token::Function(name) if name.eq_ignore_ascii_case("var") => true,
                Token::CurlyBracketBlock => {
                    block = true;
                    false
                }
                Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => false,
                _ => {
                    if self.references.stop(&self.gate) {
                        break;
                    }
                    continue;
                }
            };
            if (block && from.is_none()) || self.references.stop(&self.gate) {
                break;
            }
            let read: Result<(), Failure<'i>> = input.parse_nested_block(|inner| {
                if reference && let Some(from) = from {
                    let start = inner.state();
                    if let Ok(Token::Ident(name)) = inner.next() {
                        let to = self.references.id(name);
                        self.references.names[from].push(to);
                    }
                    inner.reset(&start);
                }
                self.read_value(inner, from);
                Ok(())
            });
            drop(read);
        }
        block
    }

    /// The most combinators in any entry of the selector list left in `input`,
    /// those inside a pseudo-class added to the entry they're in. `None` for
    /// what can't be a selector: no selector holds a `{` or a `;`.
    fn combinators<'i>(&mut self, input: &mut Parser<'i, '_>) -> Option<usize> {
        let (mut most, mut here) = (0, 0);
        let (mut started, mut gap) = (false, false);
        while let Ok(token) = input.next_including_whitespace_and_comments() {
            let nested = match token {
                Token::CurlyBracketBlock | Token::Semicolon => return None,
                Token::WhiteSpace(_) | Token::Comment(_) | Token::Delim('>' | '+' | '~') => {
                    gap = true;
                    false
                }
                Token::Comma => {
                    most = usize::max(most, here);
                    here = 0;
                    (started, gap) = (false, false);
                    false
                }
                other => {
                    let nested = matches!(
                        other,
                        Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock
                    );
                    here += usize::from(started && gap);
                    (started, gap) = (true, false);
                    nested
                }
            };
            if self.references.stop(&self.gate) {
                return None;
            }
            if nested {
                let inner: Result<Option<usize>, Failure<'i>> =
                    input.parse_nested_block(|inner| Ok(self.combinators(inner)));
                here += inner.ok().flatten()?;
            }
        }
        Some(usize::max(most, here))
    }

    fn read_body(&mut self, input: &mut Parser<'_, '_>, declarations: bool) {
        let mut inner = Reader {
            references: &mut *self.references,
            gate: self.gate,
            declarations,
            rules: true,
        };
        let mut items = RuleBodyParser::new(input, &mut inner);
        while items.next().is_some() && items.parser.references.halted.is_none() {}
    }
}

impl<'i> DeclarationParser<'i> for Reader<'_, '_> {
    type Declaration = ();
    type Error = ();

    /// A `{}` fails the declaration, whatever it holds: the parser then tries
    /// it as a rule too, so both readings are noted. Not for a `--name:`, which
    /// CSS Syntax doesn't let be a rule, and whose value is read here in full.
    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _start: &ParserState,
    ) -> Result<(), Failure<'i>> {
        let from = name.starts_with("--").then(|| self.references.id(&name));
        if self.read_value(input, from) {
            return Err(input.new_custom_error(()));
        }
        Ok(())
    }
}

impl<'i> QualifiedRuleParser<'i> for Reader<'_, '_> {
    /// Combinators in the selector, which count only if a block follows.
    type Prelude = usize;
    type QualifiedRule = ();
    type Error = ();

    fn parse_prelude<'t>(&mut self, input: &mut Parser<'i, 't>) -> Result<usize, Failure<'i>> {
        Ok(self.combinators(input).unwrap_or(0))
    }

    fn parse_block<'t>(
        &mut self,
        combinators: usize,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<(), Failure<'i>> {
        self.references.combinators = self.references.combinators.max(combinators);
        self.read_body(input, true);
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for Reader<'_, '_> {
    /// Whether the block holds declarations.
    type Prelude = bool;
    type AtRule = ();
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<bool, Failure<'i>> {
        while input.next_including_whitespace_and_comments().is_ok()
            && !self.references.stop(&self.gate)
        {}
        // `@scope` isn't on the list: it can hold declarations. Its prelude is
        // selectors, which `combinators` would have to read if it's ever on.
        let only_rules = matches!(
            name.to_ascii_lowercase().as_str(),
            "media" | "supports" | "layer" | "container" | "document" | "starting-style"
        );
        Ok(self.declarations || !only_rules)
    }

    fn rule_without_block(&mut self, _: bool, _start: &ParserState) -> Result<(), ()> {
        Ok(())
    }

    fn parse_block<'t>(
        &mut self,
        declarations: bool,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<(), Failure<'i>> {
        self.read_body(input, declarations);
        Ok(())
    }
}

impl<'i> RuleBodyItemParser<'i, (), ()> for Reader<'_, '_> {
    fn parse_declarations(&self) -> bool {
        self.declarations
    }

    fn parse_qualified(&self) -> bool {
        self.rules
    }
}
