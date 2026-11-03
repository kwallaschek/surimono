//! The generated binding traits, implemented on [`Document`]. Each method
//! is one IDL member; the spec section is in the stubs' doc comment, and the
//! algorithms stay in `document.rs` where several members share them.
//!
//! `this` and interface arguments are handles. A method turns them into
//! blitz node ids with [`Document::node_of`]; a fatal error goes through
//! [`Document::fail`] and reaches the script runner by `Document::fault`.

use blitz_dom::ns;
use surimono_webidl::runtime::{BindingError, BindingResult, ObjectId, Value};
use surimono_webidl::stubs as idl;

use crate::dispatch::interface;
use blitz_dom::QualName;

use crate::document::{self, Handle, Kind, ListKind, Object, Owned};
use crate::style_attr::{self, Declaration};
use crate::tree::dom_error;
use crate::{Document, DomError};

/// `(TrustedHTML or DOMString)` and the like: the glue always sends a string.
fn value_string(v: Value) -> String {
    match v {
        Value::String(s) => s,
        _ => String::new(),
    }
}

impl Document {
    /// Records a fatal error; the returned exception stops the script.
    pub(crate) fn fail(&mut self, e: DomError) -> BindingError {
        let message = e.script_message();
        self.fault.get_or_insert(e);
        BindingError::TypeError(message)
    }

    /// The blitz node id behind a handle.
    pub(crate) fn node_of(&mut self, this: ObjectId) -> BindingResult<usize> {
        match self.object_at(this)? {
            Object::Node(id) => Ok(id),
            // An `Attr` for a member of `Node` it has no answer to. The
            // glue's brand check lets nothing else through.
            _ => Err(BindingError::TypeError(
                "the member isn't available on this object".to_owned(),
            )),
        }
    }

    fn object_at(&mut self, this: ObjectId) -> BindingResult<Object> {
        let generation = self.generation();
        let handle = Handle::try_from(this.0).unwrap_or(0);
        match self.resolve(generation, handle) {
            Ok(o) => Ok(o.clone()),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn owner_of(&mut self, this: ObjectId) -> BindingResult<usize> {
        match self.object_at(this)? {
            Object::Tokens(n) | Object::Attributes(n) | Object::Dataset(n) | Object::Style(n) => {
                Ok(n)
            }
            _ => Err(BindingError::TypeError("wrong object".to_owned())),
        }
    }

    fn owned_object(
        &mut self,
        owner: usize,
        key: Owned,
        make: impl FnOnce() -> Object,
    ) -> BindingResult<ObjectId> {
        match self.owned_handle(owner, key, make) {
            Ok(h) => Ok(ObjectId(u64::from(h))),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn live_list(&mut self, this: ObjectId, kind: ListKind) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        let make = kind.clone();
        self.owned_object(id, Owned::List(kind), move || Object::Live(id, make))
    }

    fn attr_object(&mut self, element: usize, name: QualName) -> BindingResult<ObjectId> {
        let make = name.clone();
        self.owned_object(element, Owned::Attr(name), move || {
            Object::Attr(element, make, None)
        })
    }

    /// `(element, name, kept value)` of an `Attr`, or `None` for other
    /// objects.
    fn attr_at(
        &mut self,
        this: ObjectId,
    ) -> BindingResult<Option<(usize, QualName, Option<String>)>> {
        Ok(match self.object_at(this)? {
            Object::Attr(e, n, kept) => Some((e, n, kept)),
            _ => None,
        })
    }

    fn attr_of(&mut self, this: ObjectId) -> BindingResult<(usize, QualName, Option<String>)> {
        self.attr_at(this)?
            .ok_or_else(|| BindingError::TypeError("not an Attr".to_owned()))
    }

    fn attr_value(&mut self, this: ObjectId) -> BindingResult<String> {
        let (e, name, kept) = self.attr_of(this)?;
        Ok(self.attribute_value(e, &name).or(kept).unwrap_or_default())
    }

    /// Node of a list item: the `index`th of `this`, or `None`.
    fn list_item(&mut self, this: ObjectId, index: u32) -> BindingResult<Option<usize>> {
        let handle = Handle::try_from(this.0).unwrap_or(0);
        Ok(match self.object_at(this)? {
            Object::Static(nodes) => nodes.get(index as usize).copied(),
            Object::Live(owner, ListKind::ChildNodes) => {
                self.children(owner).get(index as usize).copied()
            }
            Object::Live(owner, kind) => self
                .live_nodes(handle, owner, &kind)
                .get(index as usize)
                .copied(),
            _ => None,
        })
    }

    fn list_length(&mut self, this: ObjectId) -> BindingResult<u32> {
        let handle = Handle::try_from(this.0).unwrap_or(0);
        let n = match self.object_at(this)? {
            Object::Static(nodes) => nodes.len(),
            Object::Live(owner, ListKind::ChildNodes) => self.children(owner).len(),
            Object::Live(owner, kind) => self.live_nodes(handle, owner, &kind).len(),
            _ => 0,
        };
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    fn query_all(&mut self, this: ObjectId, selectors: &str) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        let nodes = self.select(id, selectors, false)?;
        match self.static_list(nodes) {
            Ok(h) => Ok(ObjectId(u64::from(h))),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn query_one(&mut self, this: ObjectId, selectors: &str) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let first = self.select(id, selectors, true)?.first().copied();
        self.opt_object(first)
    }

    fn by_tag(&mut self, this: ObjectId, name: String) -> BindingResult<ObjectId> {
        self.live_list(this, ListKind::ByTag(name))
    }

    fn by_class(&mut self, this: ObjectId, names: String) -> BindingResult<ObjectId> {
        self.live_list(this, ListKind::ByClass(names))
    }

    /// `innerHTML` set: the parsed fragment replaces the children.
    fn set_inner(&mut self, id: usize, html: &str) -> BindingResult<()> {
        let context = self.parse_context(id);
        if let Some(frag) = self.parse_fragment_in(context, id, html)? {
            self.remove_all_children(id);
            self.insert_parsed(id, frag, None);
        }
        Ok(())
    }

    /// The context element `id` gives the fragment parser: itself, or a new
    /// `<body>` for what isn't an element (HTML §13.4).
    fn parse_context(&mut self, id: usize) -> usize {
        if self.is_element(id) {
            id
        } else {
            self.body().unwrap_or(id)
        }
    }

    fn parse_fragment_in(
        &mut self,
        context: usize,
        parent: usize,
        html: &str,
    ) -> BindingResult<Option<usize>> {
        self.parse_into_fragment(context, parent, html)
            .map_err(|e| self.fail(e))
    }

    fn tokens_of(&mut self, this: ObjectId) -> BindingResult<(usize, Vec<String>)> {
        let id = self.owner_of(this)?;
        let class = self.attribute(id, "class").unwrap_or_default();
        let mut tokens: Vec<String> = Vec::new();
        for t in class.split_ascii_whitespace() {
            if !tokens.iter().any(|x| x == t) {
                tokens.push(t.to_owned());
            }
        }
        Ok((id, tokens))
    }

    /// DOMTokenList "update steps" (DOM §7.1).
    fn store_tokens(&mut self, id: usize, tokens: &[String]) -> BindingResult<()> {
        if tokens.is_empty() && self.attribute(id, "class").is_none() {
            return Ok(());
        }
        Document::set_attribute(self, id, "class", &tokens.join(" ")).map_err(|e| self.fail(e))
    }

    /// `data-` attribute name for dataset property `name` (HTML §3.2.6),
    /// or `None` if `name` has a hyphen before a lower-case letter.
    fn dataset_attribute(name: &str) -> Option<String> {
        let mut out = String::from("data-");
        let mut chars = name.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '-' && chars.peek().is_some_and(char::is_ascii_lowercase) {
                return None;
            }
            if c.is_ascii_uppercase() {
                out.push('-');
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        Some(out)
    }

    /// A dataset's property for a `data-` attribute name, or `None` if it
    /// isn't one (HTML §3.2.6).
    fn dataset_property(attribute: &str) -> Option<String> {
        let rest = attribute.strip_prefix("data-")?;
        if rest.bytes().any(|b| b.is_ascii_uppercase()) {
            return None;
        }
        let mut out = String::with_capacity(rest.len());
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            match chars.peek() {
                Some(n) if c == '-' && n.is_ascii_lowercase() => {
                    out.push(n.to_ascii_uppercase());
                    chars.next();
                }
                _ => out.push(c),
            }
        }
        Some(out)
    }

    fn style_list(&mut self, this: ObjectId) -> BindingResult<(usize, Vec<Declaration>)> {
        let id = self.owner_of(this)?;
        let text = self.attribute(id, "style").unwrap_or_default();
        Ok((id, style_attr::parse(&text)))
    }

    fn store_style(&mut self, id: usize, list: &[Declaration]) -> BindingResult<()> {
        Document::set_attribute(self, id, "style", &style_attr::serialize(list))
            .map_err(|e| self.fail(e))
    }

    /// `style.fontSize`: the value, empty if unset; `None` if the name
    /// isn't a CSS property, so it stays an ordinary property.
    pub(crate) fn style_value(
        &mut self,
        this: ObjectId,
        name: String,
    ) -> BindingResult<Option<String>> {
        let Some(property) = style_attr::property_for_attribute(&name) else {
            return Ok(None);
        };
        let (_, list) = self.style_list(this)?;
        Ok(Some(style_attr::value(&list, &property)))
    }

    pub(crate) fn set_style_value(
        &mut self,
        this: ObjectId,
        name: String,
        value: String,
    ) -> BindingResult<()> {
        match style_attr::property_for_attribute(&name) {
            Some(property) => {
                idl::CSSStyleDeclaration::set_property(self, this, property, value, String::new())
            }
            None => Ok(()),
        }
    }

    /// The getter of a legacy platform object with an anonymous named
    /// getter (`DOMStringMap`): `None` for no such property.
    pub(crate) fn named_value(
        &mut self,
        this: ObjectId,
        name: String,
    ) -> BindingResult<Option<String>> {
        let id = self.owner_of(this)?;
        Ok(Document::dataset_attribute(&name).and_then(|a| self.attribute(id, &a)))
    }

    pub(crate) fn object(&mut self, id: usize) -> BindingResult<ObjectId> {
        match self.handle_for(id) {
            Ok(h) => Ok(ObjectId(u64::from(h))),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn opt_object(&mut self, id: Option<usize>) -> BindingResult<Option<ObjectId>> {
        id.map(|i| self.object(i)).transpose()
    }

    fn created(&mut self, r: Result<usize, DomError>) -> BindingResult<ObjectId> {
        match r {
            Ok(id) => self.object(id),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn is_element(&self, id: usize) -> bool {
        self.kind(id) == Some(Kind::Element)
    }

    /// The first (or last) element among `ids`.
    fn element_among(&self, ids: impl IntoIterator<Item = usize>) -> Option<usize> {
        ids.into_iter().find(|&c| self.is_element(c))
    }

    fn sibling_element(&self, mut id: usize, forward: bool) -> Option<usize> {
        while let Some(s) = self.sibling(id, forward) {
            if self.is_element(s) {
                return Some(s);
            }
            id = s;
        }
        None
    }

    fn first_element_child_of(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let found = self.element_among(self.children(id).iter().copied());
        self.opt_object(found)
    }

    fn last_element_child_of(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let found = self.element_among(self.children(id).iter().rev().copied());
        self.opt_object(found)
    }

    fn child_element_count_of(&mut self, this: ObjectId) -> BindingResult<u32> {
        let id = self.node_of(this)?;
        let n = self.children(id).iter().filter(|&&c| self.is_element(c));
        Ok(u32::try_from(n.count()).unwrap_or(u32::MAX))
    }

    fn sibling_element_of(
        &mut self,
        this: ObjectId,
        forward: bool,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let found = self.sibling_element(id, forward);
        self.opt_object(found)
    }

    /// A reflected string attribute (HTML §2.6.1): empty when absent.
    fn reflected(&mut self, this: ObjectId, name: &str) -> BindingResult<String> {
        let id = self.node_of(this)?;
        Ok(self.attribute(id, name).unwrap_or_default())
    }

    fn reflect(&mut self, this: ObjectId, name: &str, value: &str) -> BindingResult<()> {
        let id = self.node_of(this)?;
        Document::set_attribute(self, id, name, value).map_err(|e| self.fail(e))
    }

    fn opt_node(&mut self, id: Option<ObjectId>) -> BindingResult<Option<usize>> {
        id.map(|i| self.node_of(i)).transpose()
    }

    fn utf16_length(&self, id: usize) -> u32 {
        let text = Document::text_content(self, id).unwrap_or_default();
        u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX)
    }

    /// Operation 0: the handle's concrete interface, for `wrap`.
    pub(crate) fn interface_of(&mut self, this: ObjectId) -> BindingResult<u32> {
        let id = match self.object_at(this)? {
            Object::Node(id) => id,
            Object::Static(_) | Object::Live(_, ListKind::ChildNodes) => {
                return Ok(interface::NODE_LIST);
            }
            Object::Live(..) => return Ok(interface::HTML_COLLECTION),
            Object::Tokens(_) => return Ok(interface::DOM_TOKEN_LIST),
            Object::Attributes(_) => return Ok(interface::NAMED_NODE_MAP),
            Object::Dataset(_) => return Ok(interface::DOM_STRING_MAP),
            Object::Style(_) => return Ok(interface::CSS_STYLE_DECLARATION),
            Object::Attr(..) => return Ok(interface::ATTR),
        };
        Ok(match self.kind(id) {
            Some(Kind::Document) => interface::DOCUMENT,
            Some(Kind::DocumentFragment) => interface::DOCUMENT_FRAGMENT,
            Some(Kind::Text) => interface::TEXT,
            Some(Kind::Comment) => interface::COMMENT,
            Some(Kind::Element) if self.element_name(id).is_some_and(|n| n.ns == ns!(html)) => {
                interface::HTML_ELEMENT
            }
            Some(Kind::Element) => interface::ELEMENT,
            // Layout boxes, which no tree link reaches before layout.
            None => interface::NODE,
        })
    }

    /// Operation 1: an exotic object's supported property names.
    pub(crate) fn supported_names(&mut self, this: ObjectId) -> BindingResult<String> {
        let names: Vec<String> = match self.object_at(this)? {
            Object::Attributes(id) => {
                // WebIDL: HTML elements in an HTML document hide names they
                // couldn't look up (DOM §4.9.1).
                let html = self.element_name(id).is_some_and(|n| n.ns == ns!(html));
                self.attribute_names(id)
                    .into_iter()
                    .filter(|n| !html || !n.bytes().any(|b| b.is_ascii_uppercase()))
                    .collect()
            }
            Object::Dataset(id) => self
                .attribute_names(id)
                .iter()
                .filter_map(|a| Document::dataset_property(a))
                .collect(),
            Object::Live(owner, kind) if kind != ListKind::ChildNodes => {
                let handle = Handle::try_from(this.0).unwrap_or(0);
                let nodes = self.live_nodes(handle, owner, &kind).to_vec();
                let mut out: Vec<String> = Vec::new();
                for n in nodes {
                    let html = self.element_name(n).is_some_and(|q| q.ns == ns!(html));
                    let name = html.then(|| self.attribute(n, "name")).flatten();
                    for v in [self.attribute(n, "id"), name].into_iter().flatten() {
                        if !v.is_empty() && !out.contains(&v) {
                            out.push(v);
                        }
                    }
                }
                out
            }
            _ => Vec::new(),
        };
        // Newline-joined; `nameArray` in the glue splits it. A name with a
        // newline (an `id` can have one) is left out: it can't be looked up
        // by a page that follows the spec's "no whitespace in IDs" anyway.
        Ok(names
            .into_iter()
            .filter(|n| !n.contains('\n'))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

impl idl::EventTarget for Document {}

impl idl::Node for Document {
    fn node_type(&mut self, this: ObjectId) -> BindingResult<u16> {
        if self.attr_at(this)?.is_some() {
            return Ok(2);
        }
        let id = self.node_of(this)?;
        // `kind` is None only for layout boxes, which no tree link reaches
        // before layout; report them as plain nodes.
        Ok(self.kind(id).map_or(0, |k| k as u16))
    }

    fn node_name(&mut self, this: ObjectId) -> BindingResult<String> {
        if let Some((_, name, _)) = self.attr_at(this)? {
            return Ok(document::qualified_name(&name));
        }
        let id = self.node_of(this)?;
        Ok(Document::node_name(self, id))
    }

    fn is_connected(&mut self, this: ObjectId) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        Ok(Document::is_connected(self, id))
    }

    fn owner_document(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let root = self.root();
        self.opt_object((id != root).then_some(root))
    }

    fn parent_node(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        if self.attr_at(this)?.is_some() {
            return Ok(None);
        }
        let id = self.node_of(this)?;
        let p = self.parent(id);
        self.opt_object(p)
    }

    fn parent_element(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let p = self.parent(id).filter(|&p| self.is_element(p));
        self.opt_object(p)
    }

    fn has_child_nodes(&mut self, this: ObjectId) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        Ok(!self.children(id).is_empty())
    }

    fn first_child(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let c = self.children(id).first().copied();
        self.opt_object(c)
    }

    fn last_child(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let c = self.children(id).last().copied();
        self.opt_object(c)
    }

    fn previous_sibling(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let s = self.sibling(id, false);
        self.opt_object(s)
    }

    fn next_sibling(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let s = self.sibling(id, true);
        self.opt_object(s)
    }

    fn node_value(&mut self, this: ObjectId) -> BindingResult<Option<String>> {
        if self.attr_at(this)?.is_some() {
            return self.attr_value(this).map(Some);
        }
        let id = self.node_of(this)?;
        Ok(match self.kind(id) {
            Some(Kind::Text | Kind::Comment) => Document::text_content(self, id),
            _ => None,
        })
    }

    fn set_node_value(&mut self, this: ObjectId, value: Option<String>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        if matches!(self.kind(id), Some(Kind::Text | Kind::Comment)) {
            Document::set_text_content(self, id, &value.unwrap_or_default())
                .map_err(|e| self.fail(e))?;
        }
        Ok(())
    }

    fn text_content(&mut self, this: ObjectId) -> BindingResult<Option<String>> {
        if self.attr_at(this)?.is_some() {
            return self.attr_value(this).map(Some);
        }
        let id = self.node_of(this)?;
        Ok(Document::text_content(self, id))
    }

    fn set_text_content(&mut self, this: ObjectId, value: Option<String>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        Document::set_text_content(self, id, &value.unwrap_or_default()).map_err(|e| self.fail(e))
    }

    fn child_nodes(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        self.live_list(this, ListKind::ChildNodes)
    }

    fn get_root_node(&mut self, this: ObjectId, _options: Value) -> BindingResult<ObjectId> {
        let mut at = self.node_of(this)?;
        while let Some(p) = self.parent(at) {
            at = p;
        }
        self.object(at)
    }

    fn normalize(&mut self, this: ObjectId) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.normalize_tree(id)
    }

    fn clone_node(&mut self, this: ObjectId, subtree: bool) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        let copy = self.clone_tree(id, subtree)?;
        self.object(copy)
    }

    fn is_equal_node(
        &mut self,
        this: ObjectId,
        other_node: Option<ObjectId>,
    ) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        Ok(match self.opt_node(other_node)? {
            Some(other) => self.nodes_equal(id, other),
            None => false,
        })
    }

    fn is_same_node(
        &mut self,
        this: ObjectId,
        other_node: Option<ObjectId>,
    ) -> BindingResult<bool> {
        Ok(other_node == Some(this))
    }

    fn compare_document_position(&mut self, this: ObjectId, other: ObjectId) -> BindingResult<u16> {
        let id = self.node_of(this)?;
        let other = self.node_of(other)?;
        Ok(self.compare_position(id, other))
    }

    fn contains(&mut self, this: ObjectId, other: Option<ObjectId>) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        Ok(match self.opt_node(other)? {
            Some(other) => self.is_inclusive_ancestor(id, other),
            None => false,
        })
    }

    fn insert_before(
        &mut self,
        this: ObjectId,
        node: ObjectId,
        child: Option<ObjectId>,
    ) -> BindingResult<ObjectId> {
        let parent = self.node_of(this)?;
        let n = self.node_of(node)?;
        let child = self.opt_node(child)?;
        self.pre_insert(parent, n, child)?;
        Ok(node)
    }

    fn append_child(&mut self, this: ObjectId, node: ObjectId) -> BindingResult<ObjectId> {
        let parent = self.node_of(this)?;
        let n = self.node_of(node)?;
        self.pre_insert(parent, n, None)?;
        Ok(node)
    }

    fn replace_child(
        &mut self,
        this: ObjectId,
        node: ObjectId,
        child: ObjectId,
    ) -> BindingResult<ObjectId> {
        let parent = self.node_of(this)?;
        let n = self.node_of(node)?;
        let c = self.node_of(child)?;
        self.replace(parent, n, c)?;
        Ok(child)
    }

    fn remove_child(&mut self, this: ObjectId, child: ObjectId) -> BindingResult<ObjectId> {
        let parent = self.node_of(this)?;
        let c = self.node_of(child)?;
        self.pre_remove(parent, c)?;
        Ok(child)
    }
}

impl idl::Element for Document {
    fn namespace_uri(&mut self, this: ObjectId) -> BindingResult<Option<String>> {
        let id = self.node_of(this)?;
        Ok(self
            .element_name(id)
            .map(|n| n.ns.to_string())
            .filter(|n| !n.is_empty()))
    }

    fn prefix(&mut self, this: ObjectId) -> BindingResult<Option<String>> {
        let id = self.node_of(this)?;
        Ok(self
            .element_name(id)
            .and_then(|n| n.prefix.as_ref())
            .map(|p| p.to_string()))
    }

    fn local_name(&mut self, this: ObjectId) -> BindingResult<String> {
        let id = self.node_of(this)?;
        Ok(self
            .element_name(id)
            .map(|n| n.local.to_string())
            .unwrap_or_default())
    }

    fn tag_name(&mut self, this: ObjectId) -> BindingResult<String> {
        let id = self.node_of(this)?;
        Ok(Document::node_name(self, id))
    }

    fn id(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected(this, "id")
    }

    fn set_id(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "id", &value)
    }

    fn class_name(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected(this, "class")
    }

    fn set_class_name(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "class", &value)
    }

    fn class_list(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        self.owned_object(id, Owned::Tokens, move || Object::Tokens(id))
    }

    fn attributes(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        self.owned_object(id, Owned::Attributes, move || Object::Attributes(id))
    }

    fn get_attribute_names(&mut self, this: ObjectId) -> BindingResult<Vec<String>> {
        let id = self.node_of(this)?;
        Ok(self.attribute_names(id))
    }

    fn closest(&mut self, this: ObjectId, selectors: String) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let found = self.closest_match(id, &selectors)?;
        self.opt_object(found)
    }

    fn matches(&mut self, this: ObjectId, selectors: String) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        self.element_matches(id, &selectors)
    }

    fn get_elements_by_tag_name(
        &mut self,
        this: ObjectId,
        qualified_name: String,
    ) -> BindingResult<ObjectId> {
        self.by_tag(this, qualified_name)
    }

    fn get_elements_by_class_name(
        &mut self,
        this: ObjectId,
        class_names: String,
    ) -> BindingResult<ObjectId> {
        self.by_class(this, class_names)
    }

    fn query_selector(
        &mut self,
        this: ObjectId,
        selectors: String,
    ) -> BindingResult<Option<ObjectId>> {
        self.query_one(this, &selectors)
    }

    fn query_selector_all(&mut self, this: ObjectId, selectors: String) -> BindingResult<ObjectId> {
        self.query_all(this, &selectors)
    }

    fn inner_html(&mut self, this: ObjectId) -> BindingResult<Value> {
        let id = self.node_of(this)?;
        Ok(Value::String(Document::inner_html(self, id)))
    }

    fn set_inner_html(&mut self, this: ObjectId, value: Value) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.set_inner(id, &value_string(value))
    }

    fn outer_html(&mut self, this: ObjectId) -> BindingResult<Value> {
        let id = self.node_of(this)?;
        Ok(Value::String(Document::outer_html(self, id)))
    }

    fn set_outer_html(&mut self, this: ObjectId, value: Value) -> BindingResult<()> {
        let id = self.node_of(this)?;
        // HTML §8.5: no parent, nothing; the document, an error.
        let Some(parent) = self.parent(id) else {
            return Ok(());
        };
        if self.kind(parent) == Some(Kind::Document) {
            return Err(dom_error(
                "NoModificationAllowedError",
                "outerHTML: the element's parent is the document",
            ));
        }
        let context = self.parse_context(parent);
        if let Some(frag) = self.parse_fragment_in(context, parent, &value_string(value))? {
            let next = self.sibling(id, true);
            self.detach(id);
            self.insert_parsed(parent, frag, next);
        }
        Ok(())
    }

    fn insert_adjacent_html(
        &mut self,
        this: ObjectId,
        position: String,
        string: Value,
    ) -> BindingResult<()> {
        let id = self.node_of(this)?;
        let place = position.to_ascii_lowercase();
        let (parent, before) = match place.as_str() {
            "beforebegin" | "afterend" => {
                // HTML §8.5: the context must be a parent that isn't the
                // document.
                let parent = self
                    .parent(id)
                    .filter(|&p| self.kind(p) != Some(Kind::Document))
                    .ok_or_else(|| {
                        dom_error(
                            "NoModificationAllowedError",
                            "insertAdjacentHTML: the element has no parent to insert into",
                        )
                    })?;
                let before = if place == "beforebegin" {
                    Some(id)
                } else {
                    self.sibling(id, true)
                };
                (parent, before)
            }
            "afterbegin" => (id, Document::children(self, id).first().copied()),
            "beforeend" => (id, None),
            _ => {
                return Err(dom_error(
                    "SyntaxError",
                    "the position must be beforebegin, afterbegin, beforeend or afterend",
                ));
            }
        };
        let context = self.parse_context(parent);
        if let Some(frag) = self.parse_fragment_in(context, parent, &value_string(string))? {
            self.insert_parsed(parent, frag, before);
        }
        Ok(())
    }

    fn has_attributes(&mut self, this: ObjectId) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        Ok(self.attribute_count(id) > 0)
    }

    fn get_attribute(
        &mut self,
        this: ObjectId,
        qualified_name: String,
    ) -> BindingResult<Option<String>> {
        let id = self.node_of(this)?;
        Ok(self.attribute(id, &qualified_name))
    }

    fn has_attribute(&mut self, this: ObjectId, qualified_name: String) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        Ok(self.attribute(id, &qualified_name).is_some())
    }

    fn set_attribute(
        &mut self,
        this: ObjectId,
        qualified_name: String,
        value: Value,
    ) -> BindingResult<()> {
        let id = self.node_of(this)?;
        if !document::is_valid_attribute_name(&qualified_name) {
            return Err(dom_error(
                "InvalidCharacterError",
                "setAttribute: the name is not a valid attribute name",
            ));
        }
        Document::set_attribute(self, id, &qualified_name, &value_string(value))
            .map_err(|e| self.fail(e))
    }

    fn get_attribute_ns(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
        local_name: String,
    ) -> BindingResult<Option<String>> {
        let id = self.node_of(this)?;
        let ns_uri = namespace.unwrap_or_default();
        let name = self.attribute_ns(id, &ns_uri, &local_name);
        Ok(name.and_then(|n| self.attribute_value(id, &n)))
    }

    fn has_attribute_ns(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
        local_name: String,
    ) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        let ns_uri = namespace.unwrap_or_default();
        Ok(self.attribute_ns(id, &ns_uri, &local_name).is_some())
    }

    fn set_attribute_ns(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
        qualified_name: String,
        value: Value,
    ) -> BindingResult<()> {
        let id = self.node_of(this)?;
        let name = Document::validate_and_extract(namespace.as_deref(), &qualified_name, false)?;
        Document::set_attribute_ns(self, id, name, &value_string(value)).map_err(|e| self.fail(e))
    }

    fn remove_attribute_ns(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
        local_name: String,
    ) -> BindingResult<()> {
        let id = self.node_of(this)?;
        let ns_uri = namespace.unwrap_or_default();
        if let Some(name) = self.attribute_ns(id, &ns_uri, &local_name) {
            self.clear_attribute(id, name);
        }
        Ok(())
    }

    fn insert_adjacent_element(
        &mut self,
        this: ObjectId,
        where_: String,
        element: ObjectId,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let e = self.node_of(element)?;
        Ok(self.insert_adjacent(id, &where_, e)?.then_some(element))
    }

    fn insert_adjacent_text(
        &mut self,
        this: ObjectId,
        where_: String,
        data: String,
    ) -> BindingResult<()> {
        let id = self.node_of(this)?;
        let t = self.create_text(&data).map_err(|e| self.fail(e))?;
        if !self.insert_adjacent(id, &where_, t)? {
            self.drop_subtree(t);
        }
        Ok(())
    }

    fn prepend(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.prepend_nodes(id, nodes)
    }

    fn append(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.append_nodes(id, nodes)
    }

    fn replace_children(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.replace_children_with(id, nodes)
    }

    fn before(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.before_nodes(id, nodes)
    }

    fn after(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.after_nodes(id, nodes)
    }

    fn replace_with(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.replace_with_nodes(id, nodes)
    }

    fn remove(&mut self, this: ObjectId) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.detach(id);
        Ok(())
    }

    fn remove_attribute(&mut self, this: ObjectId, qualified_name: String) -> BindingResult<()> {
        let id = self.node_of(this)?;
        Document::remove_attribute(self, id, &qualified_name);
        Ok(())
    }

    fn toggle_attribute(
        &mut self,
        this: ObjectId,
        qualified_name: String,
        force: Option<bool>,
    ) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        if !document::is_valid_attribute_name(&qualified_name) {
            return Err(dom_error(
                "InvalidCharacterError",
                "toggleAttribute: the name is not a valid attribute name",
            ));
        }
        let has = self.attribute(id, &qualified_name).is_some();
        let want = force.unwrap_or(!has);
        if want && !has {
            Document::set_attribute(self, id, &qualified_name, "").map_err(|e| self.fail(e))?;
        } else if !want && has {
            Document::remove_attribute(self, id, &qualified_name);
        }
        Ok(want)
    }

    fn children(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        self.live_list(this, ListKind::Children)
    }

    fn first_element_child(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        self.first_element_child_of(this)
    }

    fn last_element_child(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        self.last_element_child_of(this)
    }

    fn child_element_count(&mut self, this: ObjectId) -> BindingResult<u32> {
        self.child_element_count_of(this)
    }

    fn previous_element_sibling(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        self.sibling_element_of(this, false)
    }

    fn next_element_sibling(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        self.sibling_element_of(this, true)
    }
}

impl idl::HTMLElement for Document {
    fn title(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected(this, "title")
    }

    fn set_title(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "title", &value)
    }

    fn lang(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected(this, "lang")
    }

    fn set_lang(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "lang", &value)
    }

    fn dataset(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        self.owned_object(id, Owned::Dataset, move || Object::Dataset(id))
    }

    fn style(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        self.owned_object(id, Owned::Style, move || Object::Style(id))
    }
}

impl idl::Document for Document {
    fn document_element(&mut self, _this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let e = Document::document_element(self);
        self.opt_object(e)
    }

    fn head(&mut self, _this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let e = Document::head(self);
        self.opt_object(e)
    }

    fn body(&mut self, _this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let e = Document::body(self);
        self.opt_object(e)
    }

    fn compat_mode(&mut self, _this: ObjectId) -> BindingResult<String> {
        Ok(if self.is_quirks() {
            "BackCompat"
        } else {
            "CSS1Compat"
        }
        .to_owned())
    }

    fn character_set(&mut self, _this: ObjectId) -> BindingResult<String> {
        Ok("UTF-8".to_owned())
    }

    fn content_type(&mut self, _this: ObjectId) -> BindingResult<String> {
        Ok("text/html".to_owned())
    }

    fn get_element_by_id(
        &mut self,
        _this: ObjectId,
        element_id: String,
    ) -> BindingResult<Option<ObjectId>> {
        let root = self.root();
        let e = self.element_by_id(root, &element_id);
        self.opt_object(e)
    }

    fn query_selector(
        &mut self,
        this: ObjectId,
        selectors: String,
    ) -> BindingResult<Option<ObjectId>> {
        self.query_one(this, &selectors)
    }

    fn query_selector_all(&mut self, this: ObjectId, selectors: String) -> BindingResult<ObjectId> {
        self.query_all(this, &selectors)
    }

    fn get_elements_by_tag_name(
        &mut self,
        this: ObjectId,
        qualified_name: String,
    ) -> BindingResult<ObjectId> {
        self.by_tag(this, qualified_name)
    }

    fn get_elements_by_class_name(
        &mut self,
        this: ObjectId,
        class_names: String,
    ) -> BindingResult<ObjectId> {
        self.by_class(this, class_names)
    }

    fn create_element_ns(
        &mut self,
        _this: ObjectId,
        namespace: Option<String>,
        qualified_name: String,
        _options: Value,
    ) -> BindingResult<ObjectId> {
        let name = Document::validate_and_extract(namespace.as_deref(), &qualified_name, true)?;
        let r = self.create_element_with(name, Vec::new());
        self.created(r)
    }

    fn create_document_fragment(&mut self, _this: ObjectId) -> BindingResult<ObjectId> {
        let r = self.create_fragment();
        self.created(r)
    }

    fn adopt_node(&mut self, _this: ObjectId, node: ObjectId) -> BindingResult<ObjectId> {
        let id = self.node_of(node)?;
        match self.kind(id) {
            Some(Kind::Document) => Err(dom_error(
                "NotSupportedError",
                "adoptNode: a document can't be adopted",
            )),
            _ => {
                self.detach(id);
                Ok(node)
            }
        }
    }

    fn prepend(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.prepend_nodes(id, nodes)
    }

    fn append(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.append_nodes(id, nodes)
    }

    fn replace_children(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.replace_children_with(id, nodes)
    }

    fn create_element(
        &mut self,
        _this: ObjectId,
        local_name: String,
        _options: Value,
    ) -> BindingResult<ObjectId> {
        match Document::create_element(self, &local_name) {
            Ok(Some(e)) => self.object(e),
            Ok(None) => Err(dom_error(
                "InvalidCharacterError",
                "createElement: the name is not a valid element name",
            )),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn create_text_node(&mut self, _this: ObjectId, data: String) -> BindingResult<ObjectId> {
        let r = self.create_text(&data);
        self.created(r)
    }

    fn create_comment(&mut self, _this: ObjectId, data: String) -> BindingResult<ObjectId> {
        let r = Document::create_comment(self, &data);
        self.created(r)
    }

    fn children(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        self.live_list(this, ListKind::Children)
    }

    fn first_element_child(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        self.first_element_child_of(this)
    }

    fn last_element_child(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        self.last_element_child_of(this)
    }

    fn child_element_count(&mut self, this: ObjectId) -> BindingResult<u32> {
        self.child_element_count_of(this)
    }
}

impl idl::DocumentFragment for Document {
    fn constructor(&mut self) -> BindingResult<ObjectId> {
        let r = self.create_fragment();
        self.created(r)
    }

    fn query_selector(
        &mut self,
        this: ObjectId,
        selectors: String,
    ) -> BindingResult<Option<ObjectId>> {
        self.query_one(this, &selectors)
    }

    fn query_selector_all(&mut self, this: ObjectId, selectors: String) -> BindingResult<ObjectId> {
        self.query_all(this, &selectors)
    }

    fn get_element_by_id(
        &mut self,
        this: ObjectId,
        element_id: String,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let e = self.element_by_id(id, &element_id);
        self.opt_object(e)
    }

    fn children(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        self.live_list(this, ListKind::Children)
    }

    fn first_element_child(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        self.first_element_child_of(this)
    }

    fn last_element_child(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        self.last_element_child_of(this)
    }

    fn child_element_count(&mut self, this: ObjectId) -> BindingResult<u32> {
        self.child_element_count_of(this)
    }

    fn prepend(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.prepend_nodes(id, nodes)
    }

    fn append(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.append_nodes(id, nodes)
    }

    fn replace_children(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.replace_children_with(id, nodes)
    }
}

impl idl::CharacterData for Document {
    fn data(&mut self, this: ObjectId) -> BindingResult<String> {
        let id = self.node_of(this)?;
        Ok(Document::text_content(self, id).unwrap_or_default())
    }

    fn set_data(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        let id = self.node_of(this)?;
        Document::set_text_content(self, id, &value).map_err(|e| self.fail(e))
    }

    fn length(&mut self, this: ObjectId) -> BindingResult<u32> {
        let id = self.node_of(this)?;
        Ok(self.utf16_length(id))
    }

    fn substring_data(&mut self, this: ObjectId, offset: u32, count: u32) -> BindingResult<String> {
        let id = self.node_of(this)?;
        Document::substring_data(self, id, offset, count)
    }

    fn append_data(&mut self, this: ObjectId, data: String) -> BindingResult<()> {
        let id = self.node_of(this)?;
        let end = self.utf16_length(id);
        Document::replace_data(self, id, end, 0, &data)
    }

    fn insert_data(&mut self, this: ObjectId, offset: u32, data: String) -> BindingResult<()> {
        let id = self.node_of(this)?;
        Document::replace_data(self, id, offset, 0, &data)
    }

    fn delete_data(&mut self, this: ObjectId, offset: u32, count: u32) -> BindingResult<()> {
        let id = self.node_of(this)?;
        Document::replace_data(self, id, offset, count, "")
    }

    fn replace_data(
        &mut self,
        this: ObjectId,
        offset: u32,
        count: u32,
        data: String,
    ) -> BindingResult<()> {
        let id = self.node_of(this)?;
        Document::replace_data(self, id, offset, count, &data)
    }

    fn previous_element_sibling(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        self.sibling_element_of(this, false)
    }

    fn next_element_sibling(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        self.sibling_element_of(this, true)
    }

    fn before(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.before_nodes(id, nodes)
    }

    fn after(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.after_nodes(id, nodes)
    }

    fn replace_with(&mut self, this: ObjectId, nodes: Vec<Value>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.replace_with_nodes(id, nodes)
    }

    fn remove(&mut self, this: ObjectId) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.detach(id);
        Ok(())
    }
}

impl idl::Text for Document {
    fn constructor(&mut self, data: String) -> BindingResult<ObjectId> {
        let r = self.create_text(&data);
        self.created(r)
    }

    fn split_text(&mut self, this: ObjectId, offset: u32) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        let new = Document::split_text(self, id, offset)?;
        self.object(new)
    }

    fn whole_text(&mut self, this: ObjectId) -> BindingResult<String> {
        let id = self.node_of(this)?;
        Ok(Document::whole_text(self, id))
    }
}

impl idl::Comment for Document {
    fn constructor(&mut self, data: String) -> BindingResult<ObjectId> {
        let r = Document::create_comment(self, &data);
        self.created(r)
    }
}

impl idl::Attr for Document {
    fn namespace_uri(&mut self, this: ObjectId) -> BindingResult<Option<String>> {
        let (_, name, _) = self.attr_of(this)?;
        Ok(Some(name.ns.to_string()).filter(|n| !n.is_empty()))
    }

    fn prefix(&mut self, this: ObjectId) -> BindingResult<Option<String>> {
        let (_, name, _) = self.attr_of(this)?;
        Ok(name.prefix.map(|p| p.to_string()))
    }

    fn local_name(&mut self, this: ObjectId) -> BindingResult<String> {
        let (_, name, _) = self.attr_of(this)?;
        Ok(name.local.to_string())
    }

    fn name(&mut self, this: ObjectId) -> BindingResult<String> {
        let (_, name, _) = self.attr_of(this)?;
        Ok(document::qualified_name(&name))
    }

    fn value(&mut self, this: ObjectId) -> BindingResult<String> {
        self.attr_value(this)
    }

    fn set_value(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        let (e, name, _) = self.attr_of(this)?;
        if self.attribute_value(e, &name).is_some() {
            self.write_attribute(e, name, &value)
                .map_err(|e| self.fail(e))?;
        } else if let Ok(h) = Handle::try_from(this.0) {
            if let Some(Object::Attr(_, _, kept)) = self.object_mut(h) {
                *kept = Some(value);
            }
        }
        Ok(())
    }

    fn owner_element(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let (e, name, _) = self.attr_of(this)?;
        let present = self.attribute_value(e, &name).is_some();
        self.opt_object(present.then_some(e))
    }

    fn specified(&mut self, _this: ObjectId) -> BindingResult<bool> {
        Ok(true)
    }
}

impl idl::NodeList for Document {
    fn item(&mut self, this: ObjectId, index: u32) -> BindingResult<Option<ObjectId>> {
        let n = self.list_item(this, index)?;
        self.opt_object(n)
    }

    fn length(&mut self, this: ObjectId) -> BindingResult<u32> {
        self.list_length(this)
    }
}

impl idl::HTMLCollection for Document {
    fn length(&mut self, this: ObjectId) -> BindingResult<u32> {
        self.list_length(this)
    }

    fn item(&mut self, this: ObjectId, index: u32) -> BindingResult<Option<ObjectId>> {
        let n = self.list_item(this, index)?;
        self.opt_object(n)
    }

    fn named_item(&mut self, this: ObjectId, name: String) -> BindingResult<Option<ObjectId>> {
        // DOM §4.2.10.2: the first element with this ID, or an HTML element
        // with this name; the empty string names nothing.
        if name.is_empty() {
            return Ok(None);
        }
        let handle = Handle::try_from(this.0).unwrap_or(0);
        let Object::Live(owner, kind) = self.object_at(this)? else {
            return Ok(None);
        };
        let nodes = self.live_nodes(handle, owner, &kind).to_vec();
        let found = nodes.into_iter().find(|&n| {
            self.attribute(n, "id").as_deref() == Some(name.as_str())
                || (self.element_name(n).is_some_and(|q| q.ns == ns!(html))
                    && self.attribute(n, "name").as_deref() == Some(name.as_str()))
        });
        self.opt_object(found)
    }
}

impl idl::NamedNodeMap for Document {
    fn length(&mut self, this: ObjectId) -> BindingResult<u32> {
        let id = self.owner_of(this)?;
        Ok(u32::try_from(self.attribute_count(id)).unwrap_or(u32::MAX))
    }

    fn item(&mut self, this: ObjectId, index: u32) -> BindingResult<Option<ObjectId>> {
        let id = self.owner_of(this)?;
        self.attribute_name_at(id, index as usize)
            .map(|n| self.attr_object(id, n))
            .transpose()
    }

    fn get_named_item(
        &mut self,
        this: ObjectId,
        qualified_name: String,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.owner_of(this)?;
        self.attribute_named(id, &qualified_name)
            .map(|n| self.attr_object(id, n))
            .transpose()
    }

    fn get_named_item_ns(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
        local_name: String,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.owner_of(this)?;
        self.attribute_ns(id, &namespace.unwrap_or_default(), &local_name)
            .map(|n| self.attr_object(id, n))
            .transpose()
    }

    fn remove_named_item(
        &mut self,
        this: ObjectId,
        qualified_name: String,
    ) -> BindingResult<ObjectId> {
        let id = self.owner_of(this)?;
        let Some(name) = self.attribute_named(id, &qualified_name) else {
            return Err(dom_error("NotFoundError", "no such attribute"));
        };
        let attr = self.attr_object(id, name.clone())?;
        self.clear_attribute(id, name);
        Ok(attr)
    }

    fn remove_named_item_ns(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
        local_name: String,
    ) -> BindingResult<ObjectId> {
        let id = self.owner_of(this)?;
        let Some(name) = self.attribute_ns(id, &namespace.unwrap_or_default(), &local_name) else {
            return Err(dom_error("NotFoundError", "no such attribute"));
        };
        let attr = self.attr_object(id, name.clone())?;
        self.clear_attribute(id, name);
        Ok(attr)
    }
}

/// DOM §7.1: a token must be non-empty and hold no ASCII whitespace.
fn check_token(token: &str) -> BindingResult<()> {
    if token.is_empty() {
        return Err(dom_error("SyntaxError", "the token is empty"));
    }
    if token.contains(['\t', '\n', '\u{c}', '\r', ' ']) {
        return Err(dom_error(
            "InvalidCharacterError",
            "the token contains whitespace",
        ));
    }
    Ok(())
}

impl idl::DOMTokenList for Document {
    fn length(&mut self, this: ObjectId) -> BindingResult<u32> {
        let (_, tokens) = self.tokens_of(this)?;
        Ok(u32::try_from(tokens.len()).unwrap_or(u32::MAX))
    }

    fn item(&mut self, this: ObjectId, index: u32) -> BindingResult<Option<String>> {
        let (_, mut tokens) = self.tokens_of(this)?;
        let i = index as usize;
        Ok((i < tokens.len()).then(|| tokens.swap_remove(i)))
    }

    fn contains(&mut self, this: ObjectId, token: String) -> BindingResult<bool> {
        let (_, tokens) = self.tokens_of(this)?;
        Ok(tokens.contains(&token))
    }

    fn add(&mut self, this: ObjectId, tokens: Vec<String>) -> BindingResult<()> {
        for t in &tokens {
            check_token(t)?;
        }
        let (id, mut list) = self.tokens_of(this)?;
        for t in tokens {
            if !list.contains(&t) {
                list.push(t);
            }
        }
        self.store_tokens(id, &list)
    }

    fn remove(&mut self, this: ObjectId, tokens: Vec<String>) -> BindingResult<()> {
        for t in &tokens {
            check_token(t)?;
        }
        let (id, mut list) = self.tokens_of(this)?;
        list.retain(|t| !tokens.contains(t));
        self.store_tokens(id, &list)
    }

    fn toggle(
        &mut self,
        this: ObjectId,
        token: String,
        force: Option<bool>,
    ) -> BindingResult<bool> {
        check_token(&token)?;
        let (id, mut list) = self.tokens_of(this)?;
        if list.contains(&token) {
            if force == Some(true) {
                return Ok(true);
            }
            list.retain(|t| *t != token);
            self.store_tokens(id, &list)?;
            return Ok(false);
        }
        if force == Some(false) {
            return Ok(false);
        }
        list.push(token);
        self.store_tokens(id, &list)?;
        Ok(true)
    }

    fn replace(&mut self, this: ObjectId, token: String, new_token: String) -> BindingResult<bool> {
        check_token(&token)?;
        check_token(&new_token)?;
        let (id, mut list) = self.tokens_of(this)?;
        let Some(at) = list.iter().position(|t| *t == token) else {
            return Ok(false);
        };
        // DOM "ordered set replace": first of the two keeps the place.
        let other = list.iter().position(|t| *t == new_token);
        list[at] = new_token.clone();
        if let Some(o) = other.filter(|&o| o != at) {
            let keep = at.min(o);
            let drop = at.max(o);
            list[keep] = new_token;
            list.remove(drop);
        }
        self.store_tokens(id, &list)?;
        Ok(true)
    }

    fn supports(&mut self, _this: ObjectId, _token: String) -> BindingResult<bool> {
        // `class` has no supported tokens (DOM §7.1).
        Err(BindingError::TypeError(
            "classList has no supported tokens".to_owned(),
        ))
    }

    fn value(&mut self, this: ObjectId) -> BindingResult<String> {
        let id = self.owner_of(this)?;
        Ok(self.attribute(id, "class").unwrap_or_default())
    }

    fn set_value(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        let id = self.owner_of(this)?;
        Document::set_attribute(self, id, "class", &value).map_err(|e| self.fail(e))
    }
}

impl idl::DOMStringMap for Document {
    fn named_setter(&mut self, this: ObjectId, name: String, value: String) -> BindingResult<()> {
        let id = self.owner_of(this)?;
        // HTML §3.2.6 "set a value".
        let Some(attribute) = Document::dataset_attribute(&name) else {
            return Err(dom_error(
                "SyntaxError",
                "dataset: a hyphen can't come before a lower-case letter",
            ));
        };
        if !document::is_valid_attribute_name(&attribute) {
            return Err(dom_error(
                "InvalidCharacterError",
                "dataset: the name is not a valid attribute name",
            ));
        }
        Document::set_attribute(self, id, &attribute, &value).map_err(|e| self.fail(e))
    }

    fn named_deleter(&mut self, this: ObjectId, name: String) -> BindingResult<()> {
        let id = self.owner_of(this)?;
        if let Some(attribute) = Document::dataset_attribute(&name) {
            Document::remove_attribute(self, id, &attribute);
        }
        Ok(())
    }
}

impl idl::CSSStyleDeclaration for Document {
    fn css_text(&mut self, this: ObjectId) -> BindingResult<String> {
        let (_, list) = self.style_list(this)?;
        Ok(style_attr::serialize(&list))
    }

    fn set_css_text(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        let id = self.owner_of(this)?;
        self.store_style(id, &style_attr::parse(&value))
    }

    fn length(&mut self, this: ObjectId) -> BindingResult<u32> {
        let (_, list) = self.style_list(this)?;
        Ok(u32::try_from(list.len()).unwrap_or(u32::MAX))
    }

    fn item(&mut self, this: ObjectId, index: u32) -> BindingResult<String> {
        let (_, list) = self.style_list(this)?;
        Ok(list
            .get(index as usize)
            .map(|d| d.name.clone())
            .unwrap_or_default())
    }

    fn get_property_value(&mut self, this: ObjectId, property: String) -> BindingResult<String> {
        let (_, list) = self.style_list(this)?;
        Ok(style_attr::value(&list, &style_attr::normalized(&property)))
    }

    fn get_property_priority(&mut self, this: ObjectId, property: String) -> BindingResult<String> {
        let (_, list) = self.style_list(this)?;
        let name = style_attr::normalized(&property);
        let important = list.iter().any(|d| d.name == name && d.important);
        Ok(if important { "important" } else { "" }.to_owned())
    }

    fn set_property(
        &mut self,
        this: ObjectId,
        property: String,
        value: String,
        priority: String,
    ) -> BindingResult<()> {
        // CSSOM §6.6 setProperty: an empty value removes; an unknown
        // priority does nothing.
        if value.is_empty() {
            return idl::CSSStyleDeclaration::remove_property(self, this, property).map(|_| ());
        }
        let important = priority.eq_ignore_ascii_case("important");
        if !priority.is_empty() && !important {
            return Ok(());
        }
        let (id, mut list) = self.style_list(this)?;
        style_attr::set(
            &mut list,
            style_attr::normalized(&property),
            value,
            important,
        );
        self.store_style(id, &list)
    }

    fn remove_property(&mut self, this: ObjectId, property: String) -> BindingResult<String> {
        let (id, mut list) = self.style_list(this)?;
        let name = style_attr::normalized(&property);
        let old = style_attr::value(&list, &name);
        if list.iter().any(|d| d.name == name) {
            list.retain(|d| d.name != name);
            self.store_style(id, &list)?;
        }
        Ok(old)
    }
}
