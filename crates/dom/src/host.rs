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
use crate::document::{self, Handle, Kind};
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
        let generation = self.generation();
        let handle = Handle::try_from(this.0).unwrap_or(0);
        self.resolve(generation, handle).map_err(|e| self.fail(e))
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
        let id = self.node_of(this)?;
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
    pub(crate) fn supported_names(&mut self, _this: ObjectId) -> BindingResult<String> {
        Ok(String::new())
    }
}

impl idl::EventTarget for Document {}

impl idl::Node for Document {
    fn node_type(&mut self, this: ObjectId) -> BindingResult<u16> {
        let id = self.node_of(this)?;
        // `kind` is None only for layout boxes, which no tree link reaches
        // before layout; report them as plain nodes.
        Ok(self.kind(id).map_or(0, |k| k as u16))
    }

    fn node_name(&mut self, this: ObjectId) -> BindingResult<String> {
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
        let id = self.node_of(this)?;
        Ok(Document::text_content(self, id))
    }

    fn set_text_content(&mut self, this: ObjectId, value: Option<String>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        Document::set_text_content(self, id, &value.unwrap_or_default()).map_err(|e| self.fail(e))
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

    fn get_element_by_id(
        &mut self,
        this: ObjectId,
        element_id: String,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let e = self.element_by_id(id, &element_id);
        self.opt_object(e)
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
