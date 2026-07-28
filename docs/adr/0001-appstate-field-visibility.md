# AppState keeps wide field visibility within tui::*

`AppState` (`src/tui/mod.rs`) has ~60 fields, mostly `pub`, read and written directly from every sibling file in `src/tui` (`keys.rs`, `dispatch.rs`, `bg.rs`, `render.rs`, `palette.rs`, `view_model.rs`). We considered narrowing this — private fields behind per-screen accessor methods — after a 2026-07 architecture review flagged it as the shared substrate under several other findings (oversized `Jobs::absorb`, `Screen` dispatch scattered across registries, duplicated palette/key guards, `render.rs` reaching into `AppState`).

We decided **not** to encapsulate it. All four of those concrete issues (#286–#289) were fully scoped and resolved during grilling without touching `AppState`'s field visibility — the friction traced back to specific, local problems, not to the field surface itself. `tui::*` is one trust domain (single binary, one author/review context per PR), so Rust module-level privacy wouldn't buy an enforced boundary that isn't already enforced by review. Encapsulating ~60 fields preemptively would be a large, low-value diff chasing a discomfort rather than a demonstrated defect.

**Revisit if**: a specific bug is traced to unconstrained cross-file mutation of a particular field — narrow that field with an accessor at that point, not preemptively across the whole struct.
