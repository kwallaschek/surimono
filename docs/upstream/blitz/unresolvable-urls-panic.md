# Draft: URLs that can't be resolved panic while building the document

For Kay to file on DioxusLabs/blitz. Not filed. Seen in blitz-dom 0.2.4. A page's own HTML is enough to reach all three; they are untrusted input for anyone rendering other people's HTML.

## Summary

1. `BaseDocument::resolve_url` (`src/document.rs`) panics when `join` fails:

   ```rust
   self.url.resolve_relative(raw).unwrap_or_else(|| {
       panic!("to be able to resolve {raw} with the base_url: {:?}", *self.url)
   })
   ```

   It runs for `<img src>` and `<link rel=stylesheet href>` as they join the document. The default base when `DocumentConfig::base_url` is `None` is a `data:` URL, which can't be a base, so without a configured base every relative URL panics. With one, `http://[` still does.

2. `StylesheetLoader::request_stylesheet` (`src/net.rs`) unwraps a CSS `@import`'s URL:

   ```rust
   let url = import.url.url().unwrap();
   ```

   `url()` is `None` when the URL didn't resolve. This fires for `<style>` text, on detached elements too, because the mutator parses style text when a style element or its children change.

3. `DocumentMutator::unload_stylesheet` (`src/mutator.rs`) has two `unreachable!()`s. `clear_attribute` calls it for a `<link>`'s `href` whether or not a stylesheet was loaded, so removing `href` from any link that hasn't loaded one panics.

## Reproduction

```rust
let doc = HtmlDocument::from_html(r#"<img src="x.png">"#, DocumentConfig::default());
let doc = HtmlDocument::from_html(r#"<style>@import "http://[";</style>"#, DocumentConfig::default());
```

For the third, `create_element` a `link`, `set_attribute` `href`, then `clear_attribute` `href`.

## Suggested fix

Skip the load when the URL doesn't resolve, as a browser does: `resolve_url` returns `Option`; `request_stylesheet` returns the import rule without fetching; `clear_attribute` unloads only when `special_data` is a stylesheet.

## Our workaround

`surimono-dom` keeps blitz-dom's nodes out of "in document" so the load hooks don't run, keeps `<style>` text in its own table, and removes a link's `href` directly. See `crates/dom/DESIGN.md`. fulgur parses our output with blitz-dom and may meet the same panics; we haven't tested that.
