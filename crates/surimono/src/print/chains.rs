//! Custom properties that name each other, and how long selectors are, as the
//! CSS parser's own rules and declarations have them. Running after the token
//! screen, which has already bounded nesting.

use std::collections::HashMap;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};

/// Which custom properties name which, and the longest selector, over a page.
#[derive(Debug, Default)]
pub(super) struct References {
    ids: HashMap<String, usize>,
    /// By id: the ids its declarations name, with repeats.
    names: Vec<Vec<usize>>,
    /// The most combinators in one selector.
    pub(super) combinators: usize,
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
    pub(super) fn read_sheet(&mut self, css: &str) {
        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        let mut reader = Reader {
            references: self,
            declarations: false,
            rules: true,
        };
        for _ in StyleSheetParser::new(&mut parser, &mut reader) {}
    }

    /// Reads a `style` attribute.
    pub(super) fn read_attribute(&mut self, css: &str) {
        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        let mut reader = Reader {
            references: self,
            declarations: true,
            rules: false,
        };
        for _ in RuleBodyParser::new(&mut parser, &mut reader) {}
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
struct Reader<'r> {
    references: &'r mut References,
    /// Declarations can be here, as in a style rule or an attribute.
    declarations: bool,
    rules: bool,
}

impl Reader<'_> {
    /// Whether a `{}` is in what's left of `input`, at its own level. `from` is
    /// the custom property the value belongs to, if it does.
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
                _ => continue,
            };
            let Some(from) = from else {
                continue;
            };
            let read: Result<(), Failure<'i>> = input.parse_nested_block(|inner| {
                if reference {
                    let start = inner.state();
                    if let Ok(Token::Ident(name)) = inner.next() {
                        let to = self.references.id(name);
                        self.references.names[from].push(to);
                    }
                    inner.reset(&start);
                }
                self.read_value(inner, Some(from));
                Ok(())
            });
            drop(read);
        }
        block
    }

    fn read_body(&mut self, input: &mut Parser<'_, '_>, declarations: bool) {
        let mut inner = Reader {
            references: &mut *self.references,
            declarations,
            rules: true,
        };
        for _ in RuleBodyParser::new(input, &mut inner) {}
    }
}

/// The most combinators in any entry of the selector list left in `input`, those
/// inside a pseudo-class added to the entry they're in.
fn combinators<'i>(input: &mut Parser<'i, '_>) -> usize {
    let (mut most, mut here) = (0, 0);
    let (mut started, mut gap) = (false, false);
    while let Ok(token) = input.next_including_whitespace_and_comments() {
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) | Token::Delim('>' | '+' | '~') => gap = true,
            Token::Comma => {
                most = usize::max(most, here);
                here = 0;
                (started, gap) = (false, false);
            }
            other => {
                let nested = matches!(
                    other,
                    Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock
                );
                here += usize::from(started && gap);
                (started, gap) = (true, false);
                if nested {
                    let inner: Result<usize, Failure<'i>> =
                        input.parse_nested_block(|inner| Ok(combinators(inner)));
                    here += inner.unwrap_or(0);
                }
            }
        }
    }
    usize::max(most, here)
}

impl<'i> DeclarationParser<'i> for Reader<'_> {
    type Declaration = ();
    type Error = ();

    /// A `{}` fails the declaration, whatever it holds: the parser then tries
    /// it as a rule too, so both readings are noted.
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

impl<'i> QualifiedRuleParser<'i> for Reader<'_> {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = ();

    fn parse_prelude<'t>(&mut self, input: &mut Parser<'i, 't>) -> Result<(), Failure<'i>> {
        let found = combinators(input);
        self.references.combinators = self.references.combinators.max(found);
        Ok(())
    }

    fn parse_block<'t>(
        &mut self,
        (): (),
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<(), Failure<'i>> {
        self.read_body(input, true);
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for Reader<'_> {
    /// Whether the block holds declarations.
    type Prelude = bool;
    type AtRule = ();
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<bool, Failure<'i>> {
        while input.next_including_whitespace_and_comments().is_ok() {}
        let only_rules = matches!(
            name.to_ascii_lowercase().as_str(),
            "media" | "supports" | "layer" | "container" | "document" | "scope" | "starting-style"
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

impl<'i> RuleBodyItemParser<'i, (), ()> for Reader<'_> {
    fn parse_declarations(&self) -> bool {
        self.declarations
    }

    fn parse_qualified(&self) -> bool {
        self.rules
    }
}
