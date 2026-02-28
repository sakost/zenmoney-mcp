# Justfile for zenmoney-mcp
# Install just: cargo install just

# Default recipe: show available commands
default:
    @just --list

# Build the project
build:
    cargo build

# Fast compile check
check:
    cargo check

# Run tests
test:
    cargo test

# Run clippy lints
lint:
    cargo clippy

# Format code (nightly for full feature set)
fmt:
    cargo +nightly fmt

# Check formatting (nightly for full feature set)
fmt-check:
    cargo +nightly fmt --check

# Run cargo-deny checks (advisories, licenses, bans, sources)
deny:
    cargo deny check

# Check for unused dependencies
machete:
    cargo machete

# Detect copy-paste code
jscpd:
    npx jscpd src/

# Full pre-commit suite
check-all: fmt-check lint deny machete test jscpd
