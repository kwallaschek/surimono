# Alpine.js licence note

Alpine.js 3.17.5 is MIT (`"license": "MIT"`, `"author": "Caleb Porzio"` in the
package's `package.json`), but its npm tarball carries no licence file, nor do
the `@alpinejs/*` plugin tarballs checked. MIT's condition applies to copies, so
the upstream `LICENSE.md` (https://github.com/alpinejs/alpine, commit
8d3d9a2f98385d4c879f4b5be080a0221279ed4d) has to be here before this repository
goes public. Task P1-CF-32 tracks it, and `verify.py --release` fails until it is
in and pinned. No licence text was invented.

`cdn.min.js` is the unmodified npm file, used by a test fixture only. It inlines
`@vue/reactivity` and `@vue/shared` (MIT, Yuxi (Evan) You), whose notice is
`LICENSE-vue-reactivity`, from the `@vue/reactivity` 3.5.40 tarball.

When the file arrives: save it here as `LICENSE.md`, run `verify.py`, which
checks that it is an MIT text, then replace `[library.license_pending]` with a
`[[library.vendored]]` entry carrying its sha256 and source URL.
