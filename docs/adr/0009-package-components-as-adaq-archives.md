# Package Components as .adaq archives

ADAQ packages each local Component as one `.adaq` ZIP containing `manifest.json` and `component.wasm`. The manifest uses an immutable UUID Component identity plus SemVer, declares the exact Component kind, ABI, parameters, inputs, outputs, dependencies, and warmup, and records the WASM SHA-256; importing a different hash for the same identity and version fails. A device may deduplicate identical package bytes, but Component Library visibility and execution remain User-scoped, and every Run freezes exact versions and hashes in its Component Lock.
