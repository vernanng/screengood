# AGENTS.md

## Commands

- `cargo build --release` — build the project
- `cargo test` — run the full test suite

## Code Map

- `src` — application source
- `docs` — project documentation

## Conventions

- Write Rust 2021 code with `use` declarations at the top of each file.
- Put unit tests in an inline `#[cfg(test)] mod tests` inside the source file they cover.
