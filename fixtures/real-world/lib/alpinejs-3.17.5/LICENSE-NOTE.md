# Alpine.js licence note

Alpine.js 3.17.5 is MIT (`"license": "MIT"`, `"author": "Caleb Porzio"` in the
package's `package.json`), but its npm tarball carries no licence file, nor do
the `@alpinejs/*` plugin tarballs checked. So `LICENSE.md` here is the upstream
file, unmodified, from https://github.com/alpinejs/alpine at commit
8d3d9a2f98385d4c879f4b5be080a0221279ed4d, the library's `source_commit` in
`manifest.toml`. The manifest pins its sha256, and `verify.py` compares it with
the source when run with network.

`cdn.min.js` is the unmodified npm file, used by a test fixture only. It inlines
`@vue/reactivity` and `@vue/shared` (MIT, Yuxi (Evan) You), whose notice is
`LICENSE-vue-reactivity`, from the `@vue/reactivity` 3.5.40 tarball.
