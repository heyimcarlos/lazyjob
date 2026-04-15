# LazyJob Agent Guidelines

## Specifications
**IMPORTANT:** Before implementing any feature, consult the specification in `specs/README.md`

- **Assume NOT implemented:** Many specs describe planned features that may not be implemented yet.
- **Check the codebase first:** Before concluding something is or isn't implemented, search for actual code. Specs describe intent; code describes reality.
- **Use Specs as guideline:** When implementing a feature, follow the design patterns, types, and architecture defined in the relevant spec.
- **Spec index:** `specs/README.md` lists all features. Each feature has its own directory under `specs/<feature>/` containing `spec.md`, `impl-plan.md`, and `lessons.md`.
- **Keep index updated:** When creating new specs or impl plans, add an entry to `specs/README.md`.

## Building with Cargo
Use cargo for quick iteration.

- **Build:** `cargo build`
- **Test all:** `cargo test`
- **Lint:** `cargo clippy -- -D warnings`
- **Format:** `cargo fmt --all`

## Code Style
- **No comments** unless code is complex and requires context for future developers.
- **Errors:** Use thiserror for error enums, anyhow for propagation. Define Result<T> type aliases.
- **Imports:** Group std, external crates, then internal lazyjob-* crates.
- **Naming:** snake_case for functions/variables, PascalCase for types, SCREAMING_CASE for constants.
- **Dead end**: If a pattern for an implementation is new, use this as a guide `./specs/rust-patterns.md` and report to the user of the usage of the specific pattern and why it is or isn't the best approach for modern Rust.  

