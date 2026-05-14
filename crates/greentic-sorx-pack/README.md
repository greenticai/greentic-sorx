# greentic-sorx-pack

Minimal SoRLa `.gtpack` loader, inspector, and doctor support for SORX.

This crate intentionally validates only the runtime handoff contract needed by
SORX. It should be replaced with a small stable `greentic-pack`/SoRLa runtime
reader API if one becomes available.

Optional ontology handoff assets are loaded and statically validated when
present:

- `assets/sorla/ontology.graph.json`
- `assets/sorla/ontology.ir.cbor`
- `assets/sorla/retrieval-bindings.json`

The loader keeps ontology support backwards compatible: packs without ontology
continue through the existing doctor and inspect paths.
