# Contributing to zenmoney-mcp

Thank you for your interest in contributing! This document explains how to get started.

## Getting started

1. Fork the repository and clone your fork
2. Install the required tooling (see [Requirements](#requirements))
3. Create a feature branch from `master`
4. Make your changes
5. Run the full check suite: `just check-all`
6. Open a pull request

## Requirements

- **Rust 1.93+** — pinned via `rust-toolchain.toml`, installed automatically by rustup
- **[just](https://github.com/casey/just)** — task runner (`cargo install just`)
- **[cargo-deny](https://github.com/EmbarkStudios/cargo-deny)** — dependency audit (`cargo install cargo-deny`)
- **[cargo-machete](https://github.com/bnjbvr/cargo-machete)** — unused dependency detection (`cargo install cargo-machete`)

## Code style

This project enforces strict linting. All code must pass the following before merging:

```sh
cargo +nightly fmt --check    # Formatting (nightly rustfmt)
cargo clippy --all-features   # Clippy (pedantic + nursery + many restriction lints)
cargo test --all-features     # Tests
cargo deny check              # License and advisory audit
```

Or equivalently: `just check-all`

### Key rules

- **No `.unwrap()` or `.expect()`** in non-test code. Use `Result` and `?`. (Tests are exempt.)
- **No `unsafe`** unless absolutely unavoidable, with justification documented.
- **No `#[allow(...)]`** without an accompanying `reason = "..."`.
- **No `todo!()`, `unimplemented!()`, `dbg!()`** in committed code.
- **No dead code** — remove unused functions, imports, variables.
- Prefer **borrowing over cloning**, **iterators over manual loops**.
- Use **strong typing** — newtype wrappers and enums over raw primitives.
- Keep `pub` visibility to the minimum necessary.

## Testing

Run the full test suite:

```sh
just test
# or
cargo test --all-features
```

## Commit messages

This project uses [Conventional Commits](https://www.conventionalcommits.org/) for changelog generation via [release-plz](https://release-plz.ieni.dev/). Every commit message must follow this format:

```
<type>(<optional scope>): <description>

[optional body]

[optional footer(s)]
```

### Allowed types

| Type | When to use | Changelog |
|------|-------------|-----------|
| `feat` | New feature or functionality | Yes |
| `fix` | Bug fix | Yes |
| `docs` | Documentation only | Yes |
| `perf` | Performance improvement | Yes |
| `refactor` | Code change that neither fixes a bug nor adds a feature | Yes |
| `build` | Build system or dependency changes | Yes |
| `ci` | CI configuration changes | Yes |
| `test` | Adding or updating tests | No |
| `chore` | Maintenance tasks (formatting, tooling) | No |

### Description style

Start the description with a **capital letter** and **do not end with a period**.

### Examples

```
feat: Add update_transaction tool

fix: Handle empty tag list in list_transactions

docs: Update tool descriptions in README

refactor(response): Extract common lookup logic into helper

feat!: Rename sync tool to incremental_sync
```

### Breaking changes

Append `!` after the type (or scope) to indicate a breaking change:

```
feat!: Rename list_transactions filter parameters
```

Or add a `BREAKING CHANGE:` footer in the commit body.

## Pull request process

1. Ensure all CI checks pass (format, clippy, test, cargo-deny)
2. Add tests for new functionality
3. Keep commits focused — one logical change per commit
4. Use conventional commit messages (see above)
5. Update documentation if the public API changes

## Reporting issues

Use the [GitHub issue tracker](https://github.com/sakost/zenmoney-mcp/issues). Please include:

- Rust version (`rustc --version`)
- Crate version
- Minimal reproduction steps
- Expected vs actual behavior

## License

By contributing, you agree that your contributions will be licensed under the same terms as the project: MIT OR Apache-2.0.
