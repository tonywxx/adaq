# Local patch

This is `adaq-trading-crypto` 1.1.0 with one Windows build fix applied.

The published build script emits absolute adapter paths inside generated Rust
string literals. On Windows, unescaped backslashes make the generated source
fail to parse. `build.rs` now formats each path as a Rust debug string literal,
which escapes platform path separators correctly.

Remove this patch and the `[patch.crates-io]` entry after the fix is available
in a published upstream release.
