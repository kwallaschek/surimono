# Draft: `append_children` loses a node moved within its own parent

For Kay to file on DioxusLabs/blitz. Not filed. Seen in blitz-dom 0.2.4, `src/mutator.rs`.

## Summary

`DocumentMutator::append_children` (and `insert_nodes_before`, through `add_children_to_parent`) handles a child that already has a parent by adding it to the new parent's list, then removing it from the old parent's list by id:

```rust
insert_children_fn(new_parent, child_ids);

for child_id in child_ids.iter().copied() {
    let child = &mut self.doc.nodes[child_id];
    let old_parent_id = child.parent.replace(parent_id);
    // ...
    if let Some(old_parent_id) = old_parent_id {
        let old_parent = &mut self.doc.nodes[old_parent_id];
        // ...
        old_parent.children.retain(|id| *id != child_id);
```

When old and new parent are the same, `retain` removes both entries.

## Effect

The node stays in the slab, its `parent` still names the parent, and the parent no longer lists it. A later subtree drop can't reach it, so its `parent` link dangles, and indexing the tree with it panics (`invalid key`) or reaches an unrelated node once the slot is reused.

## Reproduction

```rust
let mut doc = BaseDocument::new(DocumentConfig::default());
let mut m = doc.mutate();
let p = m.create_element(qual_name!("div", html), vec![]);
let a = m.create_element(qual_name!("b", html), vec![]);
let b = m.create_element(qual_name!("i", html), vec![]);
m.append_children(p, &[a, b]);
m.append_children(p, &[a]); // the DOM's p.appendChild(p.firstChild)
assert_eq!(m.child_ids(p), vec![b, a]); // fails: [b]
```

## Suggested fix

Remove the child from its old parent before inserting, so the order of the two steps can't matter. `insert_nodes_before` also needs the anchor's index computed after that removal.

## Our workaround

`surimono-dom` calls `remove_node` first, so blitz-dom only sees parentless nodes.
