//! The generated binding traits, implemented on [`Document`]. Each method
//! is one IDL member; the spec section is in the stubs' doc comment, and the
//! algorithms stay in `document.rs` where several members share them.
//!
//! `this` and interface arguments are handles. A method turns them into
//! blitz node ids with [`Document::node_of`]; a fatal error goes through
//! [`Document::fail`] and reaches the script runner by `Document::fault`.

use std::collections::HashSet;

use blitz_dom::ns;
use surimono_webidl::runtime::{BindingError, BindingResult, ObjectId, Value};
use surimono_webidl::stubs as idl;

use crate::dispatch::interface;
use blitz_dom::QualName;

use crate::document::{
    self, APPLICATION_XML, Handle, Kind, ListKind, MutationRecord, Object, Owned, Special,
    TEXT_HTML,
};
use crate::style_attr::{self, Declaration};
use crate::tree::{dom_error, timed_out};
use crate::{Document, DomError, LayoutPolicy};

/// `(TrustedHTML or DOMString)` and the like: the glue always sends a string.
fn value_string(v: Value) -> String {
    match v {
        Value::String(s) => s,
        _ => String::new(),
    }
}

/// HTML §3.2.2 "element interface", from the element index (§ Index) and the
/// obsolete elements of §16.
fn html_interface(local: &str) -> u32 {
    match local {
        "a" => interface::HTML_ANCHOR_ELEMENT,
        "area" => interface::HTML_AREA_ELEMENT,
        "audio" => interface::HTML_AUDIO_ELEMENT,
        "base" => interface::HTML_BASE_ELEMENT,
        "blockquote" | "q" => interface::HTML_QUOTE_ELEMENT,
        "body" => interface::HTML_BODY_ELEMENT,
        "br" => interface::HTMLBR_ELEMENT,
        "button" => interface::HTML_BUTTON_ELEMENT,
        "canvas" => interface::HTML_CANVAS_ELEMENT,
        "caption" => interface::HTML_TABLE_CAPTION_ELEMENT,
        "col" | "colgroup" => interface::HTML_TABLE_COL_ELEMENT,
        "data" => interface::HTML_DATA_ELEMENT,
        "datalist" => interface::HTML_DATA_LIST_ELEMENT,
        "del" | "ins" => interface::HTML_MOD_ELEMENT,
        "details" => interface::HTML_DETAILS_ELEMENT,
        "dialog" => interface::HTML_DIALOG_ELEMENT,
        "dir" => interface::HTML_DIRECTORY_ELEMENT,
        "div" => interface::HTML_DIV_ELEMENT,
        "dl" => interface::HTMLD_LIST_ELEMENT,
        "embed" => interface::HTML_EMBED_ELEMENT,
        "fieldset" => interface::HTML_FIELD_SET_ELEMENT,
        "font" => interface::HTML_FONT_ELEMENT,
        "form" => interface::HTML_FORM_ELEMENT,
        "frame" => interface::HTML_FRAME_ELEMENT,
        "frameset" => interface::HTML_FRAME_SET_ELEMENT,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => interface::HTML_HEADING_ELEMENT,
        "head" => interface::HTML_HEAD_ELEMENT,
        "hr" => interface::HTMLHR_ELEMENT,
        "html" => interface::HTML_HTML_ELEMENT,
        "iframe" => interface::HTMLI_FRAME_ELEMENT,
        "img" => interface::HTML_IMAGE_ELEMENT,
        "input" => interface::HTML_INPUT_ELEMENT,
        "label" => interface::HTML_LABEL_ELEMENT,
        "legend" => interface::HTML_LEGEND_ELEMENT,
        "li" => interface::HTMLLI_ELEMENT,
        "link" => interface::HTML_LINK_ELEMENT,
        "map" => interface::HTML_MAP_ELEMENT,
        "marquee" => interface::HTML_MARQUEE_ELEMENT,
        "menu" => interface::HTML_MENU_ELEMENT,
        "meta" => interface::HTML_META_ELEMENT,
        "meter" => interface::HTML_METER_ELEMENT,
        "object" => interface::HTML_OBJECT_ELEMENT,
        "ol" => interface::HTMLO_LIST_ELEMENT,
        "optgroup" => interface::HTML_OPT_GROUP_ELEMENT,
        "option" => interface::HTML_OPTION_ELEMENT,
        "output" => interface::HTML_OUTPUT_ELEMENT,
        "p" => interface::HTML_PARAGRAPH_ELEMENT,
        "param" => interface::HTML_PARAM_ELEMENT,
        "picture" => interface::HTML_PICTURE_ELEMENT,
        "pre" | "listing" | "xmp" => interface::HTML_PRE_ELEMENT,
        "progress" => interface::HTML_PROGRESS_ELEMENT,
        "script" => interface::HTML_SCRIPT_ELEMENT,
        "select" => interface::HTML_SELECT_ELEMENT,
        "selectedcontent" => interface::HTML_SELECTED_CONTENT_ELEMENT,
        "slot" => interface::HTML_SLOT_ELEMENT,
        "source" => interface::HTML_SOURCE_ELEMENT,
        "span" => interface::HTML_SPAN_ELEMENT,
        "style" => interface::HTML_STYLE_ELEMENT,
        "table" => interface::HTML_TABLE_ELEMENT,
        "tbody" | "tfoot" | "thead" => interface::HTML_TABLE_SECTION_ELEMENT,
        "td" | "th" => interface::HTML_TABLE_CELL_ELEMENT,
        "template" => interface::HTML_TEMPLATE_ELEMENT,
        "textarea" => interface::HTML_TEXT_AREA_ELEMENT,
        "time" => interface::HTML_TIME_ELEMENT,
        "title" => interface::HTML_TITLE_ELEMENT,
        "tr" => interface::HTML_TABLE_ROW_ELEMENT,
        "track" => interface::HTML_TRACK_ELEMENT,
        "ul" => interface::HTMLU_LIST_ELEMENT,
        "video" => interface::HTML_VIDEO_ELEMENT,
        "abbr" | "acronym" | "address" | "article" | "aside" | "b" | "basefont" | "bdi" | "bdo"
        | "big" | "center" | "cite" | "code" | "dd" | "dfn" | "dt" | "em" | "figcaption"
        | "figure" | "footer" | "header" | "hgroup" | "i" | "kbd" | "main" | "mark" | "nav"
        | "nobr" | "noembed" | "noframes" | "noscript" | "plaintext" | "rb" | "rp" | "rt"
        | "rtc" | "ruby" | "s" | "samp" | "search" | "section" | "small" | "strike" | "strong"
        | "sub" | "summary" | "sup" | "tt" | "u" | "var" | "wbr" => interface::HTML_ELEMENT,
        name if custom_element_name(name) => interface::HTML_ELEMENT,
        _ => interface::HTML_UNKNOWN_ELEMENT,
    }
}

/// HTML §4.13.3 "valid custom element name", as far as it matters here: a
/// name with a hyphen, starting with a lowercase ASCII letter, with no
/// uppercase ASCII letter, and not one of the reserved names.
pub(crate) fn custom_element_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.contains('-')
        && !name.contains(|c: char| c.is_ascii_uppercase())
        && !matches!(
            name,
            "annotation-xml"
                | "color-profile"
                | "font-face"
                | "font-face-src"
                | "font-face-uri"
                | "font-face-format"
                | "font-face-name"
                | "missing-glyph"
        )
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

    pub(crate) fn object_at(&mut self, this: ObjectId) -> BindingResult<Object> {
        let generation = self.generation();
        let handle = Handle::try_from(this.0).unwrap_or(0);
        match self.resolve(generation, handle) {
            Ok(o) => Ok(o.clone()),
            Err(e) => Err(self.fail(e)),
        }
    }

    /// `f` on the static list `this` is, if it is one, without copying it: a
    /// loop over `list[i]` must not copy the list per item.
    fn with_static_list<R>(
        &mut self,
        this: ObjectId,
        f: impl FnOnce(&[usize]) -> R,
    ) -> BindingResult<Option<R>> {
        let generation = self.generation();
        let handle = Handle::try_from(this.0).unwrap_or(0);
        match self.resolve(generation, handle) {
            Ok(Object::Static(nodes)) => Ok(Some(f(nodes))),
            Ok(_) => Ok(None),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn owner_of(&mut self, this: ObjectId) -> BindingResult<usize> {
        match self.object_at(this)? {
            Object::Tokens(n) | Object::Attributes(n) | Object::Dataset(n) | Object::Style(n) => {
                Ok(n)
            }
            Object::Computed(_) => Err(dom_error(
                "NoModificationAllowedError",
                "a computed style can't be changed",
            )),
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
        Ok(kept
            .or_else(|| self.attribute_value(e, &name))
            .unwrap_or_default())
    }

    /// Node of a list item: the `index`th of `this`, or `None`.
    fn list_item(&mut self, this: ObjectId, index: u32) -> BindingResult<Option<usize>> {
        let index = index as usize;
        if let Some(n) = self.with_static_list(this, |nodes| nodes.get(index).copied())? {
            return Ok(n);
        }
        let handle = Handle::try_from(this.0).unwrap_or(0);
        Ok(match self.object_at(this)? {
            Object::Live(owner, ListKind::ChildNodes) => self.children(owner).get(index).copied(),
            Object::Live(owner, kind) => self.live_nodes(handle, owner, &kind)?.get(index).copied(),
            _ => None,
        })
    }

    fn list_length(&mut self, this: ObjectId) -> BindingResult<u32> {
        if let Some(n) = self.with_static_list(this, <[usize]>::len)? {
            return Ok(u32::try_from(n).unwrap_or(u32::MAX));
        }
        let handle = Handle::try_from(this.0).unwrap_or(0);
        let n = match self.object_at(this)? {
            Object::Live(owner, ListKind::ChildNodes) => self.children(owner).len(),
            Object::Live(owner, kind) => self.live_nodes(handle, owner, &kind)?.len(),
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

    fn by_tag_ns(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
        local: String,
    ) -> BindingResult<ObjectId> {
        // "*" is any namespace; null and "" are no namespace.
        let namespace = match namespace.as_deref() {
            Some("*") => None,
            Some(n) => Some(n.to_owned()),
            None => Some(String::new()),
        };
        self.live_list(this, ListKind::ByTagNs(namespace, local))
    }

    fn by_class(&mut self, this: ObjectId, names: String) -> BindingResult<ObjectId> {
        self.live_list(this, ListKind::ByClass(names))
    }

    /// `innerHTML` set: the parsed fragment replaces the children.
    fn set_inner(&mut self, id: usize, html: &str) -> BindingResult<()> {
        let context = self.parse_context(id);
        if let Some(frag) = self.parse_fragment_in(context, id, html)? {
            // HTML §8.5: a template's innerHTML is its content.
            let top = if self.is_html_template(id) {
                self.template_content(id).map_err(|e| self.fail(e))?
            } else {
                id
            };
            self.replace_all_parsed(top, frag);
        }
        Ok(())
    }

    /// Parses `html` into a fragment fit for `parent`, in the context of
    /// `context`, or of a new `<body>` element if there is none. In an XML
    /// document, as XML (DOM Parsing §2.1): input that isn't well-formed is a
    /// `SyntaxError`.
    pub(crate) fn parse_fragment_in(
        &mut self,
        context: Option<usize>,
        parent: usize,
        html: &str,
    ) -> BindingResult<Option<usize>> {
        if !self.in_html_document(parent) {
            return match self.parse_xml_fragment(context.unwrap_or(parent), parent, html) {
                Ok(frag) => Ok(Some(frag)),
                Err(crate::xml::Stop::Limit(e)) => Err(e),
                // Only `Document::parse_xml_stoppable` passes a stop.
                Err(crate::xml::Stop::Halted) => Err(timed_out()),
                Err(crate::xml::Stop::Malformed(m)) => Err(BindingError::DomException {
                    name: "SyntaxError",
                    message: m,
                }),
            };
        }
        let made = match context {
            Some(_) => None,
            None => {
                let body = QualName::new(None, ns!(html), blitz_dom::local_name!("body"));
                let made = self.create_element_with(body, Vec::new());
                Some(made.map_err(|e| self.fail(e))?)
            }
        };
        let parsed = match context.or(made) {
            Some(c) => self
                .parse_into_fragment(c, parent, html)
                .map_err(|e| self.fail(e)),
            None => Ok(None),
        };
        if let Some(m) = made {
            self.drop_subtree(m);
        }
        parsed
    }

    /// The parse context for what goes under `id` in `outerHTML` and
    /// `insertAdjacentHTML` (HTML §8.5): `id` unless it isn't an element.
    fn parse_context(&self, id: usize) -> Option<usize> {
        self.is_element(id).then_some(id)
    }

    /// The set of `class` tokens, in order; linear in their number.
    fn tokens_of(&mut self, this: ObjectId) -> BindingResult<(usize, Vec<String>)> {
        let id = self.owner_of(this)?;
        let class = self.attribute(id, "class").unwrap_or_default();
        let mut seen = HashSet::new();
        let mut tokens: Vec<String> = Vec::new();
        for t in class.split_ascii_whitespace() {
            self.poll()?;
            if seen.insert(t) {
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
        if let Object::Computed(id) = self.object_at(this)? {
            return Ok((id, Vec::new()));
        }
        let id = self.owner_of(this)?;
        let text = self.attribute(id, "style").unwrap_or_default();
        Ok((id, self.parse_style(&text)?))
    }

    /// CSSOM §6.6: a computed style's "computed flag".
    fn check_writable(&mut self, this: ObjectId) -> BindingResult<()> {
        self.owner_of(this).map(|_| ())
    }

    fn parse_style(&mut self, text: &str) -> BindingResult<Vec<Declaration>> {
        style_attr::parse(text, self.quirks_mode(), self.deadline.as_ref())
            .map_err(|stop| self.style_stopped(stop))
    }

    fn style_stopped(&mut self, stop: style_attr::Stop) -> BindingError {
        match stop {
            style_attr::Stop::TooLarge => self.fail(style_attr::limit_error()),
            style_attr::Stop::TimeLimit => timed_out(),
        }
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

    /// A reflected URL attribute (HTML §2.6.1): the value parsed against the
    /// document's base URL, or as written if that fails.
    fn reflected_url(&mut self, this: ObjectId, name: &str) -> BindingResult<String> {
        let id = self.node_of(this)?;
        let Some(value) = self.attribute(id, name) else {
            return Ok(String::new());
        };
        let base = self.base_url(self.owner_of_node(id));
        let parsed = url::Url::parse(&base).and_then(|b| b.join(&value));
        Ok(parsed
            .or_else(|_| url::Url::parse(&value))
            .map_or(value, String::from))
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
            Object::Style(_) | Object::Computed(_) => {
                return Ok(interface::CSS_STYLE_DECLARATION);
            }
            Object::Implementation(_) => return Ok(interface::DOM_IMPLEMENTATION),
            Object::Record(_) => return Ok(interface::MUTATION_RECORD),
            Object::Attr(..) => return Ok(interface::ATTR),
            Object::Range(_) => return Ok(interface::RANGE),
            Object::StaticRange(_) => return Ok(interface::STATIC_RANGE),
        };
        Ok(match self.kind(id) {
            Some(Kind::Document) if self.info(id).xml_interface => interface::XML_DOCUMENT,
            Some(Kind::Document) => interface::DOCUMENT,
            Some(Kind::DocumentFragment) => interface::DOCUMENT_FRAGMENT,
            Some(Kind::DocumentType) => interface::DOCUMENT_TYPE,
            Some(Kind::Text) if self.is_cdata(id) => interface::CDATA_SECTION,
            Some(Kind::Text) => interface::TEXT,
            Some(Kind::Comment) => interface::COMMENT,
            Some(Kind::ProcessingInstruction) => interface::PROCESSING_INSTRUCTION,
            Some(Kind::Element) => match self.element_name(id) {
                Some(n) if n.ns == ns!(html) => html_interface(n.local.as_ref()),
                Some(n) if n.ns == ns!(svg) => interface::SVG_ELEMENT,
                _ => interface::ELEMENT,
            },
            // Layout boxes, which no tree link reaches before layout.
            None => interface::NODE,
        })
    }

    /// What §4.4's namespace lookups start from: `this`, or an `Attr`'s
    /// element, if it has one.
    fn lookup_node(&mut self, this: ObjectId) -> BindingResult<Option<usize>> {
        if let Some((e, _, kept)) = self.attr_at(this)? {
            return Ok(kept.is_none().then_some(e));
        }
        self.node_of(this).map(Some)
    }

    /// DOM §4.9 "set an attribute": `attr`, free or already on `element`,
    /// goes on `element`, in the place of any with its namespace and local
    /// name, which is returned.
    fn set_attr_node(&mut self, element: usize, attr: ObjectId) -> BindingResult<Option<ObjectId>> {
        let (on, name, kept) = self.attr_of(attr)?;
        let Some(value) = kept else {
            if on != element {
                return Err(dom_error(
                    "InUseAttributeError",
                    "the attribute is already on another element",
                ));
            }
            return Ok(Some(attr));
        };
        let old = self.attribute_ns(element, name.ns.as_ref(), name.local.as_ref());
        let old_object = old
            .clone()
            .map(|n| self.attr_object(element, n))
            .transpose()?;
        let handle = Handle::try_from(attr.0).unwrap_or(0);
        self.attach_attr(handle, element, name, value, old)
            .map_err(|e| self.fail(e))?;
        Ok(old_object)
    }

    /// The `Attr` `attr`, on `element` as far as a script can tell, or an
    /// error if it isn't one.
    fn attached_attr(&mut self, element: usize, attr: ObjectId) -> BindingResult<QualName> {
        match self.attr_of(attr)? {
            (on, name, None) if on == element => Ok(name),
            _ => Err(dom_error(
                "NotFoundError",
                "the attribute isn't on this element",
            )),
        }
    }

    /// (name, public ID, system ID) of the doctype `this`.
    fn doctype_field(&mut self, this: ObjectId) -> BindingResult<(String, String, String)> {
        let id = self.node_of(this)?;
        Ok(match self.special(id) {
            Some(Special::Doctype {
                name,
                public_id,
                system_id,
            }) => (name.clone(), public_id.clone(), system_id.clone()),
            _ => Default::default(),
        })
    }

    /// DOM §4.5.1 `createHTMLDocument`: the tree in the new document `doc`,
    /// which is being made in.
    fn fill_html_document(&mut self, doc: usize, title: Option<&str>) -> Result<(), DomError> {
        let doctype = self.create_doctype("html", "", "")?;
        self.place_quietly(doc, &[doctype], None);
        let element = |d: &mut Self, name: &str| {
            d.create_element(name)
                .and_then(|e| e.ok_or(DomError::NoDocument))
        };
        let html = element(self, "html")?;
        self.place_quietly(doc, &[html], None);
        let head = element(self, "head")?;
        self.place_quietly(html, &[head], None);
        if let Some(title) = title {
            let t = element(self, "title")?;
            self.place_quietly(head, &[t], None);
            let text = self.create_text(title)?;
            self.place_quietly(t, &[text], None);
        }
        let body = element(self, "body")?;
        self.place_quietly(html, &[body], None);
        Ok(())
    }

    /// `DOMParser.parseFromString`: the new document's handle.
    pub(crate) fn parse_from_string(
        &mut self,
        this: ObjectId,
        text: String,
        content_type: String,
    ) -> BindingResult<u32> {
        let parser_document = self.node_of(this)?;
        let url = self.info(parser_document).url.clone();
        let parsed = match content_type.as_str() {
            "text/html" => self.parse_html_document(&text, url),
            "text/xml" => self.parse_xml_document(&text, "text/xml", url),
            "application/xml" => self.parse_xml_document(&text, APPLICATION_XML, url),
            "image/svg+xml" => self.parse_xml_document(&text, "image/svg+xml", url),
            _ => self.parse_xml_document(&text, "application/xhtml+xml", url),
        };
        let Some(doc) = parsed.map_err(|e| self.fail(e))? else {
            return Err(timed_out());
        };
        let h = self.object(doc)?;
        Ok(Handle::try_from(h.0).unwrap_or(0))
    }

    /// One of `Location`'s readings of document `this`'s URL (HTML §7.2.4).
    pub(crate) fn url_part(&mut self, this: ObjectId, part: String) -> BindingResult<String> {
        let doc = self.node_of(this)?;
        let text = Document::url(self, doc);
        if part == "href" {
            return Ok(text.to_owned());
        }
        let Ok(url) = url::Url::parse(text) else {
            return Ok(String::new());
        };
        let prefixed = |sign: char, s: Option<&str>| match s {
            Some(s) if !s.is_empty() => format!("{sign}{s}"),
            _ => String::new(),
        };
        let host = url.host_str().unwrap_or_default();
        Ok(match part.as_str() {
            "origin" => url.origin().ascii_serialization(),
            "protocol" => format!("{}:", url.scheme()),
            "host" => match url.port() {
                Some(p) => format!("{host}:{p}"),
                None => host.to_owned(),
            },
            "hostname" => host.to_owned(),
            "port" => url.port().map_or_else(String::new, |p| p.to_string()),
            "pathname" => url.path().to_owned(),
            "search" => prefixed('?', url.query()),
            "hash" => prefixed('#', url.fragment()),
            _ => String::new(),
        })
    }

    /// `text` resolved against document `this`'s base URL, or empty.
    pub(crate) fn url_join(&mut self, this: ObjectId, text: String) -> BindingResult<String> {
        let doc = self.node_of(this)?;
        let base = self.base_url(doc);
        let joined = url::Url::parse(&base).and_then(|b| b.join(&text));
        let url = joined.or_else(|_| url::Url::parse(&text));
        Ok(url
            .map(String::from)
            .ok()
            .filter(|u| u.len() <= document::MAX_URL_BYTES)
            .unwrap_or_default())
    }

    /// Document `this`'s URL with fragment `value`, a leading `#` dropped:
    /// `location.hash`'s setter (HTML §7.2.4). Empty if it has no URL, the
    /// fragment is the one it has, a missing one counting as empty, or the
    /// result is over [`MAX_URL_BYTES`](document::MAX_URL_BYTES).
    pub(crate) fn url_fragment(&mut self, this: ObjectId, value: String) -> BindingResult<String> {
        let doc = self.node_of(this)?;
        let Ok(mut url) = url::Url::parse(Document::url(self, doc)) else {
            return Ok(String::new());
        };
        let old = url.fragment().unwrap_or_default().to_owned();
        url.set_fragment(Some(value.strip_prefix('#').unwrap_or(&value)));
        let fragment = url.fragment().unwrap_or_default();
        if fragment == old || url.as_str().len() > document::MAX_URL_BYTES {
            return Ok(String::new());
        }
        Ok(url.into())
    }

    /// `text` resolved against `this`'s base URL, if the document's URL
    /// "can have its URL rewritten" to it (HTML §7.2.5), else empty.
    pub(crate) fn url_rewrite(&mut self, this: ObjectId, text: String) -> BindingResult<String> {
        let doc = self.node_of(this)?;
        let target = self.url_join(this, text)?;
        let (Ok(from), Ok(to)) = (
            url::Url::parse(Document::url(self, doc)),
            url::Url::parse(&target),
        ) else {
            return Ok(String::new());
        };
        let same_origin = from.scheme() == to.scheme()
            && from.username() == to.username()
            && from.password() == to.password()
            && from.host() == to.host()
            && from.port() == to.port();
        let allowed = same_origin
            && (matches!(to.scheme(), "http" | "https")
                || (to.scheme() == "file" && from.path() == to.path())
                || (from.path() == to.path() && from.query() == to.query()));
        Ok(if allowed && target.len() <= document::MAX_URL_BYTES {
            target
        } else {
            String::new()
        })
    }

    /// Document `this`'s URL becomes `url`, which [`url_rewrite`](Self::url_rewrite)
    /// or a fragment gave; no content type changes with it.
    pub(crate) fn set_document_url(&mut self, this: ObjectId, url: String) -> BindingResult<()> {
        let doc = self.node_of(this)?;
        if url.len() > document::MAX_URL_BYTES {
            return Err(dom_error("SecurityError", "the URL is too long"));
        }
        self.write_url(doc, &url).map_err(|e| self.fail(e))
    }

    /// Parses one queued frame; whether there was one to parse.
    pub(crate) fn load_frame(&mut self) -> BindingResult<bool> {
        match self.load_next_frame().map_err(|e| self.fail(e))? {
            None => Ok(false),
            Some(true) => Ok(true),
            Some(false) => Err(timed_out()),
        }
    }

    /// `node` as XML, for `XMLSerializer` and XML documents' `innerHTML`.
    fn xml_text(&mut self, node: usize) -> BindingResult<String> {
        match crate::xml::serialize(self, node) {
            Ok(text) => Ok(text),
            Err(crate::xml::Refused::Time(e)) => Err(e),
            Err(crate::xml::Refused::TooLong(limit)) => {
                Err(self.fail(DomError::TextLimit { limit }))
            }
        }
    }

    /// The next `<iframe>` whose `load` is due; 0 for none.
    pub(crate) fn next_frame_load(&mut self) -> BindingResult<u32> {
        Ok(self.next_frame())
    }

    /// Whether the document `this` is an iframe's.
    pub(crate) fn is_frame_document(&mut self, this: ObjectId) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        Ok(self.info(id).frame)
    }

    /// Whether frame document `this` still has its `<iframe>` in a page.
    pub(crate) fn frame_attached(&mut self, this: ObjectId) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        Ok(self.frame_is_attached(id))
    }

    /// `XMLSerializer.serializeToString`.
    pub(crate) fn serialize_xml(&mut self, node: ObjectId) -> BindingResult<String> {
        let id = self.node_of(node)?;
        self.xml_text(id)
    }

    /// Whether node `this` is in the page's document.
    pub(crate) fn in_page(&mut self, this: ObjectId) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        Ok(self.owner_of_node(id) == self.root())
    }

    /// A layout query by `member`: nothing under [`LayoutPolicy::Zeros`],
    /// the call's end under [`LayoutPolicy::Throw`].
    pub(crate) fn layout_query(&mut self, member: String) -> BindingResult<()> {
        match self.layout {
            LayoutPolicy::Zeros => Ok(()),
            LayoutPolicy::Throw => Err(self.fail(DomError::LayoutQuery { member })),
        }
    }

    /// `getComputedStyle(this)`'s handle, one per element.
    pub(crate) fn computed_style(&mut self, this: ObjectId) -> BindingResult<u32> {
        self.layout_query("getComputedStyle".to_owned())?;
        let id = self.node_of(this)?;
        let h = self.owned_object(id, Owned::Computed, move || Object::Computed(id))?;
        Ok(Handle::try_from(h.0).unwrap_or(0))
    }

    /// The element whose `on*` attributes are the window's (HTML §8.1.7.1): the
    /// body or frameset element whose attribute was written last, or the
    /// document's body element. 0 for none.
    pub(crate) fn handler_element(&mut self) -> BindingResult<u32> {
        if let Some(h) = self.handler_source {
            return Ok(h);
        }
        match Document::body(self, self.root()) {
            Some(id) => self.handle_for(id).map_err(|e| self.fail(e)),
            None => Ok(0),
        }
    }

    /// Every element's `id` in tree order, each followed by a newline, for
    /// named access on the window (HTML §7.3.3). The tree has at most
    /// `max_nodes` nodes, and the text is the document's.
    pub(crate) fn id_names(&mut self) -> BindingResult<String> {
        let mut names = String::new();
        let mut stack = vec![self.root()];
        while let Some(at) = stack.pop() {
            self.poll()?;
            stack.extend(self.children(at).iter().rev().copied());
            if let Some(id) = self.attribute(at, "id") {
                names.push_str(&id);
                names.push('\n');
            }
        }
        Ok(names)
    }

    /// The handles of the page's `svg` elements, newline-joined, in the order
    /// of their end tags.
    pub(crate) fn svg_elements(&mut self) -> BindingResult<String> {
        let mut found = Vec::new();
        // (node, its children are done).
        let mut stack = vec![(self.root(), false)];
        while let Some((at, done)) = stack.pop() {
            self.poll()?;
            let svg = self
                .element_name(at)
                .is_some_and(|n| n.ns == ns!(svg) && n.local.as_ref() == "svg");
            if done {
                found.push(at);
                continue;
            }
            if svg {
                stack.push((at, true));
            }
            stack.extend(self.children(at).iter().rev().map(|&c| (c, false)));
        }
        let mut out = String::new();
        for n in found {
            let h = self.object(n)?;
            out.push_str(&h.0.to_string());
            out.push('\n');
        }
        Ok(out)
    }

    /// A label's labeled control (HTML §4.10.4), or 0: with `for`, the first
    /// element of the label's own tree with that `id`, if it is labelable;
    /// otherwise the first labelable descendant. Polls the deadline.
    pub(crate) fn labeled_control(&mut self, this: ObjectId) -> BindingResult<u32> {
        let label = self.node_of(this)?;
        let wanted = self.attribute(label, "for");
        let mut top = label;
        if wanted.is_some() {
            while let Some(p) = self.parent(top) {
                top = p;
            }
        }
        let mut found = None;
        let mut stack = vec![top];
        while let Some(at) = stack.pop() {
            self.poll()?;
            stack.extend(self.children(at).iter().rev().copied());
            match &wanted {
                Some(id) if !id.is_empty() && self.attribute(at, "id").as_ref() == Some(id) => {
                    found = Some(at).filter(|&c| self.labelable(c));
                    break;
                }
                None if at != label && self.labelable(at) => {
                    found = Some(at);
                    break;
                }
                _ => {}
            }
        }
        match found {
            Some(c) => self.handle_for(c).map_err(|e| self.fail(e)),
            None => Ok(0),
        }
    }

    /// HTML §4.10.2, labelable elements. Form-associated custom elements
    /// aren't there yet.
    fn labelable(&self, id: usize) -> bool {
        self.element_name(id).is_some_and(|n| {
            n.ns == ns!(html)
                && match n.local.as_ref() {
                    "button" | "meter" | "output" | "progress" | "select" | "textarea" => true,
                    "input" => !self
                        .attribute(id, "type")
                        .is_some_and(|t| t.eq_ignore_ascii_case("hidden")),
                    _ => false,
                }
        })
    }

    /// Radio input activation (HTML §4.10.5.1.15): checks `this`, unchecks the
    /// rest of its group, and returns the handle of the one that was checked,
    /// or 0, for undoing it. `set_checked` on `true` does the same.
    pub(crate) fn radio_check(&mut self, this: ObjectId) -> BindingResult<u32> {
        let id = self.node_of(this)?;
        let other = self.uncheck_group(id)?;
        Document::set_attribute(self, id, "checked", "").map_err(|e| self.fail(e))?;
        match other {
            Some(o) => self.handle_for(o).map_err(|e| self.fail(e)),
            None => Ok(0),
        }
    }

    /// Unchecks the other radio buttons of `id`'s group (HTML §4.10.5.1.15: same
    /// tree, same form owner, same non-empty name); returns the first that was
    /// checked. Polls the deadline: its length is the tree's.
    fn uncheck_group(&mut self, id: usize) -> BindingResult<Option<usize>> {
        if self
            .attribute(id, "type")
            .is_none_or(|t| !t.eq_ignore_ascii_case("radio"))
        {
            return Ok(None);
        }
        let Some(name) = self.attribute(id, "name").filter(|n| !n.is_empty()) else {
            return Ok(None);
        };
        let owner = self.radio_owner(id);
        let mut top = id;
        while let Some(p) = self.parent(top) {
            top = p;
        }
        let mut found = None;
        let mut stack = vec![top];
        while let Some(at) = stack.pop() {
            self.poll()?;
            stack.extend(self.children(at).iter().rev().copied());
            if at == id || !self.is_radio_in(at, &name) || self.radio_owner(at) != owner {
                continue;
            }
            if self.attribute(at, "checked").is_some() {
                found.get_or_insert(at);
                Document::remove_attribute(self, at, "checked");
            }
        }
        Ok(found)
    }

    fn is_radio_in(&self, id: usize, name: &str) -> bool {
        self.element_name(id)
            .is_some_and(|n| n.ns == ns!(html) && n.local.as_ref() == "input")
            && self
                .attribute(id, "type")
                .is_some_and(|t| t.eq_ignore_ascii_case("radio"))
            && self.attribute(id, "name").is_some_and(|n| n == name)
    }

    /// What identifies a radio button's form owner: its `form` attribute's
    /// text, or its nearest ancestor form.
    fn radio_owner(&self, id: usize) -> (Option<String>, Option<usize>) {
        if let Some(f) = self.attribute(id, "form") {
            return (Some(f), None);
        }
        let mut at = self.parent(id);
        while let Some(p) = at {
            if self
                .element_name(p)
                .is_some_and(|n| n.ns == ns!(html) && n.local.as_ref() == "form")
            {
                return (None, Some(p));
            }
            at = self.parent(p);
        }
        (None, None)
    }

    pub(crate) fn observer_new(&mut self) -> BindingResult<u32> {
        Ok(self.new_observer())
    }

    pub(crate) fn observe(
        &mut self,
        observer: u32,
        target: ObjectId,
        flags: u32,
        names: String,
    ) -> BindingResult<()> {
        let id = self.node_of(target)?;
        self.add_registration(observer, id, flags, &names)
            .map_err(|e| self.fail(e))
    }

    pub(crate) fn observer_disconnect(&mut self, observer: u32) -> BindingResult<()> {
        self.remove_registrations(observer);
        Ok(())
    }

    /// The observer's next record's handle, 0 for none.
    pub(crate) fn observer_take(&mut self, observer: u32) -> BindingResult<u32> {
        Ok(self.next_record(observer).unwrap_or(0))
    }

    pub(crate) fn records_pending(&mut self) -> BindingResult<bool> {
        Ok(self.has_records())
    }

    fn record_of(&mut self, this: ObjectId) -> BindingResult<Box<MutationRecord>> {
        match self.object_at(this)? {
            Object::Record(r) => Ok(r),
            _ => Err(BindingError::TypeError("not a MutationRecord".to_owned())),
        }
    }

    /// Operation 2: the page lifecycle moves `readyState`.
    pub(crate) fn set_ready_state(&mut self, state: u32) -> BindingResult<()> {
        self.set_ready(state);
        Ok(())
    }

    /// Operation 1: an exotic object's supported property names.
    pub(crate) fn supported_names(&mut self, this: ObjectId) -> BindingResult<String> {
        let names: Vec<String> = match self.object_at(this)? {
            Object::Attributes(id) => {
                // WebIDL: HTML elements in an HTML document hide names they
                // couldn't look up (DOM §4.9.1).
                let html = self.element_name(id).is_some_and(|n| n.ns == ns!(html))
                    && self.in_html_document(id);
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
                let nodes = self.live_nodes(handle, owner, &kind)?.to_vec();
                let mut out: Vec<String> = Vec::new();
                for n in nodes {
                    self.poll()?;
                    let html = self.element_name(n).is_some_and(|q| q.ns == ns!(html));
                    let name = html.then(|| self.attribute(n, "name")).flatten();
                    out.extend(self.attribute(n, "id"));
                    out.extend(name);
                }
                out.retain(|v| !v.is_empty());
                out
            }
            _ => Vec::new(),
        };
        // Each name ends in a newline, so one empty name (a sole `data-`)
        // isn't an empty list; `nameList` in the glue splits it. A name with
        // a newline (an `id` can have one) is left out: it can't be looked
        // up by a page that follows the spec's "no whitespace in IDs" anyway.
        // Ordered sets: duplicates would make a proxy's `ownKeys` throw.
        let mut seen = HashSet::new();
        let mut out = String::new();
        for name in &names {
            self.poll()?;
            if !name.contains('\n') && seen.insert(name.as_str()) {
                out.push_str(name);
                out.push('\n');
            }
        }
        Ok(out)
    }
}

impl idl::EventTarget for Document {}

impl idl::Node for Document {
    fn node_type(&mut self, this: ObjectId) -> BindingResult<u16> {
        if self.attr_at(this)?.is_some() {
            return Ok(2);
        }
        let id = self.node_of(this)?;
        if self.is_cdata(id) {
            return Ok(4);
        }
        // `kind` is None only for layout boxes, which no tree link reaches
        // before layout; report them as plain nodes.
        Ok(self.kind(id).map_or(0, |k| k as u16))
    }

    fn base_uri(&mut self, this: ObjectId) -> BindingResult<String> {
        let node = match self.attr_at(this)? {
            Some((e, ..)) => e,
            None => self.node_of(this)?,
        };
        Ok(self.base_url(self.owner_of_node(node)))
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
        if let Some((e, ..)) = self.attr_at(this)? {
            let owner = self.owner_of_node(e);
            return self.opt_object(Some(owner));
        }
        let id = self.node_of(this)?;
        let owner = self.owner_of_node(id);
        self.opt_object((self.kind(id) != Some(Kind::Document)).then_some(owner))
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
            Some(Kind::Text | Kind::Comment | Kind::ProcessingInstruction) => {
                Document::text_content(self, id)
            }
            _ => None,
        })
    }

    fn set_node_value(&mut self, this: ObjectId, value: Option<String>) -> BindingResult<()> {
        let id = self.node_of(this)?;
        if matches!(
            self.kind(id),
            Some(Kind::Text | Kind::Comment | Kind::ProcessingInstruction)
        ) {
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

    fn lookup_prefix(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
    ) -> BindingResult<Option<String>> {
        let id = self.lookup_node(this)?;
        match namespace.filter(|n| !n.is_empty()) {
            Some(n) => id.map_or(Ok(None), |i| self.locate_prefix(i, &n)),
            None => Ok(None),
        }
    }

    fn lookup_namespace_uri(
        &mut self,
        this: ObjectId,
        prefix: Option<String>,
    ) -> BindingResult<Option<String>> {
        let id = self.lookup_node(this)?;
        let prefix = prefix.filter(|p| !p.is_empty());
        id.map_or(Ok(None), |i| self.locate_namespace(i, prefix.as_deref()))
    }

    fn is_default_namespace(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
    ) -> BindingResult<bool> {
        let id = self.lookup_node(this)?;
        let default = id.map_or(Ok(None), |i| self.locate_namespace(i, None))?;
        Ok(default == namespace.filter(|n| !n.is_empty()))
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

    fn webkit_matches_selector(
        &mut self,
        this: ObjectId,
        selectors: String,
    ) -> BindingResult<bool> {
        idl::Element::matches(self, this, selectors)
    }

    fn get_elements_by_tag_name(
        &mut self,
        this: ObjectId,
        qualified_name: String,
    ) -> BindingResult<ObjectId> {
        self.by_tag(this, qualified_name)
    }

    fn get_elements_by_tag_name_ns(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
        local_name: String,
    ) -> BindingResult<ObjectId> {
        self.by_tag_ns(this, namespace, local_name)
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
        if !self.in_html_document(id) {
            // DOM Parsing §2.1: XML documents serialize as XML. The children
            // only: a fragment of them keeps no prefix context either.
            let top = self.template_of(id).unwrap_or(id);
            let mut out = String::new();
            for c in Document::children(self, top).to_vec() {
                out.push_str(&self.xml_text(c)?);
            }
            return Ok(Value::String(out));
        }
        Ok(Value::String(Document::inner_html(self, id)))
    }

    fn set_inner_html(&mut self, this: ObjectId, value: Value) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.set_inner(id, &value_string(value))
    }

    fn outer_html(&mut self, this: ObjectId) -> BindingResult<Value> {
        let id = self.node_of(this)?;
        if !self.in_html_document(id) {
            return self.xml_text(id).map(Value::String);
        }
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
            self.replace_with_parsed(parent, frag, id)?;
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
        // HTML §8.5: the `html` element is no context either, or the parser
        // would start before the head and add a second head and body.
        let is_root_element = self
            .element_name(parent)
            .is_some_and(|n| n.ns == ns!(html) && n.local.as_ref() == "html");
        let context = self.parse_context(parent).filter(|_| !is_root_element);
        if let Some(frag) = self.parse_fragment_in(context, parent, &value_string(string))? {
            self.insert_parsed(parent, frag, before);
        }
        Ok(())
    }

    fn has_attributes(&mut self, this: ObjectId) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        Ok(self.attribute_count(id) > 0)
    }

    fn get_attribute_node(
        &mut self,
        this: ObjectId,
        qualified_name: String,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        self.attribute_named(id, &qualified_name)
            .map(|n| self.attr_object(id, n))
            .transpose()
    }

    fn get_attribute_node_ns(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
        local_name: String,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        self.attribute_ns(id, namespace.as_deref().unwrap_or(""), &local_name)
            .map(|n| self.attr_object(id, n))
            .transpose()
    }

    fn set_attribute_node(
        &mut self,
        this: ObjectId,
        attr: ObjectId,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        self.set_attr_node(id, attr)
    }

    fn set_attribute_node_ns(
        &mut self,
        this: ObjectId,
        attr: ObjectId,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        self.set_attr_node(id, attr)
    }

    fn remove_attribute_node(&mut self, this: ObjectId, attr: ObjectId) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        let name = self.attached_attr(id, attr)?;
        self.clear_attribute(id, name);
        Ok(attr)
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
    fn constructor(&mut self) -> BindingResult<ObjectId> {
        // DOM §4.5: an XML document, whose interface is still Document.
        let info = self.blank_info(false, APPLICATION_XML);
        let r = self.new_document(info);
        self.created(r)
    }

    fn implementation(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        self.owned_object(id, Owned::Implementation, move || {
            Object::Implementation(id)
        })
    }

    fn url(&mut self, this: ObjectId) -> BindingResult<String> {
        let id = self.node_of(this)?;
        Ok(Document::url(self, id).to_owned())
    }

    fn document_uri(&mut self, this: ObjectId) -> BindingResult<String> {
        idl::Document::url(self, this)
    }

    fn doctype(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let d = self.doctype_of(id);
        self.opt_object(d)
    }

    fn document_element(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let e = self.document_element_of(id);
        self.opt_object(e)
    }

    fn head(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let e = Document::head(self, id);
        self.opt_object(e)
    }

    fn body(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let e = Document::body(self, id);
        self.opt_object(e)
    }

    fn title(&mut self, this: ObjectId) -> BindingResult<String> {
        let id = self.node_of(this)?;
        Document::title(self, id)
    }

    fn set_title(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        let id = self.node_of(this)?;
        Document::set_title(self, id, &value)
    }

    fn ready_state(&mut self, this: ObjectId) -> BindingResult<String> {
        let id = self.node_of(this)?;
        // Only the page has a lifecycle; "complete" is the others' default.
        let page = id == self.root();
        Ok(match Document::ready_state(self) {
            0 if page => "loading",
            1 if page => "interactive",
            _ => "complete",
        }
        .to_owned())
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

    fn charset(&mut self, _this: ObjectId) -> BindingResult<String> {
        Ok("UTF-8".to_owned())
    }

    fn input_encoding(&mut self, _this: ObjectId) -> BindingResult<String> {
        Ok("UTF-8".to_owned())
    }

    fn content_type(&mut self, this: ObjectId) -> BindingResult<String> {
        let id = self.node_of(this)?;
        Ok(self.info(id).content_type.to_owned())
    }

    fn create_cdata_section(&mut self, this: ObjectId, data: String) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        if self.info(id).html {
            return Err(dom_error(
                "NotSupportedError",
                "createCDATASection: an HTML document has none",
            ));
        }
        if data.contains("]]>") {
            return Err(dom_error(
                "InvalidCharacterError",
                "createCDATASection: the data contains \"]]>\"",
            ));
        }
        let r = self.create_cdata(&data);
        self.created(r)
    }

    fn create_attribute(&mut self, this: ObjectId, local_name: String) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        if !document::is_valid_attribute_name(&local_name) {
            return Err(dom_error(
                "InvalidCharacterError",
                "createAttribute: the name is not valid",
            ));
        }
        let local = if self.info(id).html {
            local_name.to_ascii_lowercase()
        } else {
            local_name
        };
        let name = QualName::new(None, ns!(), blitz_dom::LocalName::from(local));
        let h = self.free_attr(id, name).map_err(|e| self.fail(e))?;
        Ok(ObjectId(u64::from(h)))
    }

    fn create_attribute_ns(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
        qualified_name: String,
    ) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        let name = Document::validate_and_extract(namespace.as_deref(), &qualified_name, false)?;
        let h = self.free_attr(id, name).map_err(|e| self.fail(e))?;
        Ok(ObjectId(u64::from(h)))
    }

    fn create_processing_instruction(
        &mut self,
        _this: ObjectId,
        target: String,
        data: String,
    ) -> BindingResult<ObjectId> {
        if !crate::xml::is_name(&target) || data.contains("?>") {
            return Err(dom_error(
                "InvalidCharacterError",
                "createProcessingInstruction: the target or data is not valid",
            ));
        }
        let r = self.create_instruction(&target, &data);
        self.created(r)
    }

    fn get_element_by_id(
        &mut self,
        this: ObjectId,
        element_id: String,
    ) -> BindingResult<Option<ObjectId>> {
        let root = self.node_of(this)?;
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

    fn get_elements_by_tag_name_ns(
        &mut self,
        this: ObjectId,
        namespace: Option<String>,
        local_name: String,
    ) -> BindingResult<ObjectId> {
        self.by_tag_ns(this, namespace, local_name)
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

    fn import_node(
        &mut self,
        _this: ObjectId,
        node: ObjectId,
        options: Value,
    ) -> BindingResult<ObjectId> {
        let id = self.node_of(node)?;
        // §4.5: `this` is `making`, so the copy joins it.
        if self.kind(id) == Some(Kind::Document) {
            return Err(dom_error(
                "NotSupportedError",
                "importNode: a document can't be imported",
            ));
        }
        let copy = self.clone_tree(id, matches!(options, Value::Bool(true)))?;
        self.object(copy)
    }

    fn create_range(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        match self.new_range(id) {
            Ok(h) => Ok(ObjectId(u64::from(h))),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn scripts(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        self.live_list(this, ListKind::Scripts)
    }

    fn create_document_fragment(&mut self, _this: ObjectId) -> BindingResult<ObjectId> {
        let r = self.create_fragment();
        self.created(r)
    }

    fn adopt_node(&mut self, this: ObjectId, node: ObjectId) -> BindingResult<ObjectId> {
        let id = self.node_of(node)?;
        match self.kind(id) {
            Some(Kind::Document) => Err(dom_error(
                "NotSupportedError",
                "adoptNode: a document can't be adopted",
            )),
            _ => {
                let doc = self.node_of(this)?;
                self.detach(id);
                self.adopt_subtree(id, doc);
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

impl idl::XMLDocument for Document {}

impl idl::HTMLIFrameElement for Document {
    fn src(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected_url(this, "src")
    }

    fn set_src(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "src", &value)
    }

    fn srcdoc(&mut self, this: ObjectId) -> BindingResult<Value> {
        Ok(Value::String(self.reflected(this, "srcdoc")?))
    }

    fn set_srcdoc(&mut self, this: ObjectId, value: Value) -> BindingResult<()> {
        self.reflect(this, "srcdoc", &value_string(value))
    }

    fn name(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected(this, "name")
    }

    fn set_name(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "name", &value)
    }

    fn content_document(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let id = self.node_of(this)?;
        let d = self.frame_document(id).map_err(|e| self.fail(e))?;
        self.opt_object(d)
    }
}

impl idl::DOMImplementation for Document {
    fn create_document_type(
        &mut self,
        _this: ObjectId,
        name: String,
        public_id: String,
        system_id: String,
    ) -> BindingResult<ObjectId> {
        // DOM §1.5 "valid doctype name".
        if name.contains(['\t', '\n', '\u{c}', '\r', ' ', '\0', '>']) {
            return Err(dom_error(
                "InvalidCharacterError",
                "createDocumentType: the name is not valid",
            ));
        }
        let r = self.create_doctype(&name, &public_id, &system_id);
        self.created(r)
    }

    fn create_document(
        &mut self,
        _this: ObjectId,
        namespace: Option<String>,
        qualified_name: String,
        doctype: Option<ObjectId>,
    ) -> BindingResult<ObjectId> {
        let name = if qualified_name.is_empty() {
            None
        } else {
            let n = Document::validate_and_extract(namespace.as_deref(), &qualified_name, true)?;
            Some(n)
        };
        let doctype = self.opt_node(doctype)?;
        let mut info = self.blank_info(false, Document::xml_content_type(namespace.as_deref()));
        info.xml_interface = true;
        let doc = self.new_document(info).map_err(|e| self.fail(e))?;
        self.making_in(doc, |d| {
            if let Some(t) = doctype {
                d.pre_insert(doc, t, None)?;
            }
            if let Some(n) = name {
                let e = d.create_element_with(n, Vec::new());
                let e = e.map_err(|e| d.fail(e))?;
                d.pre_insert(doc, e, None)?;
            }
            Ok(())
        })?;
        self.object(doc)
    }

    fn create_html_document(
        &mut self,
        _this: ObjectId,
        title: Option<String>,
    ) -> BindingResult<ObjectId> {
        let info = self.blank_info(true, TEXT_HTML);
        let doc = self.new_document(info).map_err(|e| self.fail(e))?;
        let built = self.making_in(doc, |d| d.fill_html_document(doc, title.as_deref()));
        built.map_err(|e| self.fail(e))?;
        self.object(doc)
    }

    fn has_feature(&mut self, _this: ObjectId) -> BindingResult<bool> {
        Ok(true)
    }
}

impl idl::DocumentType for Document {
    fn name(&mut self, this: ObjectId) -> BindingResult<String> {
        Ok(self.doctype_field(this)?.0)
    }

    fn public_id(&mut self, this: ObjectId) -> BindingResult<String> {
        Ok(self.doctype_field(this)?.1)
    }

    fn system_id(&mut self, this: ObjectId) -> BindingResult<String> {
        Ok(self.doctype_field(this)?.2)
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

impl idl::ProcessingInstruction for Document {
    fn target(&mut self, this: ObjectId) -> BindingResult<String> {
        let id = self.node_of(this)?;
        Ok(match self.special(id) {
            Some(Special::Instruction { target }) => target.clone(),
            _ => String::new(),
        })
    }
}

impl idl::CDATASection for Document {}

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
        let (e, name, kept) = self.attr_of(this)?;
        let written = if kept.is_some() {
            let handle = Handle::try_from(this.0).unwrap_or(0);
            self.set_kept_value(handle, value)
        } else {
            self.write_attribute(e, name, &value)
        };
        written.map_err(|e| self.fail(e))
    }

    fn owner_element(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let (e, _, kept) = self.attr_of(this)?;
        self.opt_object(kept.is_none().then_some(e))
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
        let nodes = self.live_nodes(handle, owner, &kind)?.to_vec();
        for n in nodes {
            self.poll()?;
            if self.attribute(n, "id").as_deref() == Some(name.as_str())
                || (self.element_name(n).is_some_and(|q| q.ns == ns!(html))
                    && self.attribute(n, "name").as_deref() == Some(name.as_str()))
            {
                return self.opt_object(Some(n));
            }
        }
        Ok(None)
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

    fn set_named_item(
        &mut self,
        this: ObjectId,
        attr: ObjectId,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.owner_of(this)?;
        self.set_attr_node(id, attr)
    }

    fn set_named_item_ns(
        &mut self,
        this: ObjectId,
        attr: ObjectId,
    ) -> BindingResult<Option<ObjectId>> {
        let id = self.owner_of(this)?;
        self.set_attr_node(id, attr)
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
        let mut seen: HashSet<String> = list.iter().cloned().collect();
        for t in tokens {
            self.poll()?;
            if seen.insert(t.clone()) {
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
        let gone: HashSet<&String> = tokens.iter().collect();
        list.retain(|t| !gone.contains(t));
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

impl idl::SVGElement for Document {
    fn style(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        idl::HTMLElement::style(self, this)
    }
}

impl idl::CSSStyleDeclaration for Document {
    fn parent_rule(&mut self, _this: ObjectId) -> BindingResult<Option<ObjectId>> {
        Ok(None)
    }

    fn css_text(&mut self, this: ObjectId) -> BindingResult<String> {
        let (_, list) = self.style_list(this)?;
        Ok(style_attr::serialize(&list))
    }

    fn set_css_text(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        let id = self.owner_of(this)?;
        let list = self.parse_style(&value)?;
        self.store_style(id, &list)
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
        self.check_writable(this)?;
        // CSSOM §6.6 setProperty, in its order: an unsupported property does
        // nothing, an empty value removes, an unknown priority or an invalid
        // value does nothing.
        let name = style_attr::normalized(&property);
        if !style_attr::supported(&name) {
            return Ok(());
        }
        if value.is_empty() {
            return idl::CSSStyleDeclaration::remove_property(self, this, property).map(|_| ());
        }
        let important = priority.eq_ignore_ascii_case("important");
        if !priority.is_empty() && !important {
            return Ok(());
        }
        let parsed = style_attr::parse_value(&name, &value, self.quirks_mode());
        let Some(text) = parsed.map_err(|stop| self.style_stopped(stop))? else {
            return Ok(());
        };
        let (id, mut list) = self.style_list(this)?;
        style_attr::set(&mut list, name, text, important);
        self.store_style(id, &list)
    }

    fn remove_property(&mut self, this: ObjectId, property: String) -> BindingResult<String> {
        self.check_writable(this)?;
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

impl idl::MutationRecord for Document {
    fn type_(&mut self, this: ObjectId) -> BindingResult<String> {
        Ok(self.record_of(this)?.kind.to_owned())
    }

    fn target(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let t = self.record_of(this)?.target;
        self.object(t)
    }

    fn added_nodes(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        Ok(ObjectId(u64::from(self.record_of(this)?.added)))
    }

    fn removed_nodes(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        Ok(ObjectId(u64::from(self.record_of(this)?.removed)))
    }

    fn previous_sibling(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let n = self.record_of(this)?.previous;
        self.opt_object(n)
    }

    fn next_sibling(&mut self, this: ObjectId) -> BindingResult<Option<ObjectId>> {
        let n = self.record_of(this)?.next;
        self.opt_object(n)
    }

    fn attribute_name(&mut self, this: ObjectId) -> BindingResult<Option<String>> {
        Ok(self.record_of(this)?.attribute)
    }

    fn attribute_namespace(&mut self, this: ObjectId) -> BindingResult<Option<String>> {
        Ok(self.record_of(this)?.namespace)
    }

    fn old_value(&mut self, this: ObjectId) -> BindingResult<Option<String>> {
        Ok(self.record_of(this)?.old_value)
    }
}

impl idl::HTMLTemplateElement for Document {
    fn content(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let id = self.node_of(this)?;
        match self.template_content(id) {
            Ok(c) => self.object(c),
            Err(e) => Err(self.fail(e)),
        }
    }
}

impl idl::HTMLAreaElement for Document {
    fn href(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected_url(this, "href")
    }

    fn set_href(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "href", &value)
    }

    fn rel(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected(this, "rel")
    }

    fn set_rel(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "rel", &value)
    }
}

impl idl::HTMLAnchorElement for Document {
    fn href(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected_url(this, "href")
    }

    fn set_href(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "href", &value)
    }

    fn rel(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected(this, "rel")
    }

    fn set_rel(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "rel", &value)
    }
}

impl idl::HTMLImageElement for Document {
    fn src(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected_url(this, "src")
    }

    fn set_src(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "src", &value)
    }

    fn alt(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected(this, "alt")
    }

    fn set_alt(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "alt", &value)
    }
}

impl idl::HTMLLinkElement for Document {
    fn href(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected_url(this, "href")
    }

    fn set_href(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "href", &value)
    }

    fn rel(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected(this, "rel")
    }

    fn set_rel(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "rel", &value)
    }
}

impl idl::HTMLScriptElement for Document {
    fn src(&mut self, this: ObjectId) -> BindingResult<Value> {
        self.reflected_url(this, "src").map(Value::String)
    }

    fn set_src(&mut self, this: ObjectId, value: Value) -> BindingResult<()> {
        self.reflect(this, "src", &value_string(value))
    }

    fn type_(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected(this, "type")
    }

    fn set_type(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "type", &value)
    }
}

/// `value` and `checked` write the attribute, not the element's dirty state
/// (HTML §4.10.5): what a script fills in has to reach the printed page, and
/// serialized HTML is what gets printed.
impl idl::HTMLInputElement for Document {
    fn value(&mut self, this: ObjectId) -> BindingResult<String> {
        let value = self.reflected(this, "value")?;
        // "default/on" mode: no attribute reads as `on`.
        let id = self.node_of(this)?;
        let on = self.attribute(id, "value").is_none()
            && matches!(
                idl::HTMLInputElement::type_(self, this)?.as_str(),
                "checkbox" | "radio"
            );
        Ok(if on { "on".to_owned() } else { value })
    }

    fn set_value(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "value", &value)
    }

    fn type_(&mut self, this: ObjectId) -> BindingResult<String> {
        // HTML §4.10.5: limited to known values, else text.
        let t = self.reflected(this, "type")?.to_ascii_lowercase();
        const KNOWN: &[&str] = &[
            "hidden",
            "text",
            "search",
            "tel",
            "url",
            "email",
            "password",
            "date",
            "month",
            "week",
            "time",
            "datetime-local",
            "number",
            "range",
            "color",
            "checkbox",
            "radio",
            "file",
            "submit",
            "image",
            "reset",
            "button",
        ];
        Ok(if KNOWN.contains(&t.as_str()) {
            t
        } else {
            "text".to_owned()
        })
    }

    fn set_type(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "type", &value)
    }

    fn name(&mut self, this: ObjectId) -> BindingResult<String> {
        self.reflected(this, "name")
    }

    fn set_name(&mut self, this: ObjectId, value: String) -> BindingResult<()> {
        self.reflect(this, "name", &value)
    }

    fn checked(&mut self, this: ObjectId) -> BindingResult<bool> {
        let id = self.node_of(this)?;
        Ok(self.attribute(id, "checked").is_some())
    }

    fn set_checked(&mut self, this: ObjectId, value: bool) -> BindingResult<()> {
        let id = self.node_of(this)?;
        if value {
            self.uncheck_group(id)?;
            Document::set_attribute(self, id, "checked", "").map_err(|e| self.fail(e))
        } else {
            Document::remove_attribute(self, id, "checked");
            Ok(())
        }
    }
}
