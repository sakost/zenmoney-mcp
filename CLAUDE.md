# ZenMoney MCP Server — Development Guidelines

## Project Overview

MCP server wrapping the `zenmoney-rs` crate, exposing ZenMoney personal finance API as MCP tools for LLM assistants. Uses `rmcp` with stdio transport.

## Build & Test

```bash
just build        # cargo build
just check        # cargo check (fast compile check)
just test         # cargo test
just lint         # cargo clippy
just fmt          # cargo +nightly fmt
just fmt-check    # cargo +nightly fmt --check
just deny         # cargo deny check
just machete      # check for unused deps
just jscpd        # copy-paste detection
just check-all    # full pre-commit suite
```

## Architecture

- `src/main.rs` — Entry point: env, tracing to stderr, auto-sync, stdio serve
- `src/server.rs` — `ZenMoneyMcpServer` struct with `#[tool_router]` and `#[tool_handler]`
- `src/params.rs` — Parameter structs (`#[derive(Deserialize, JsonSchema)]`)
- `src/response.rs` — Enriched output structs (resolve IDs to names)

## Coding Standards

- Follow all lint rules from `Cargo.toml` (copied from `zenmoney-rs`)
- No `unwrap()`, `expect()`, `panic!()`, `todo!()`, `unimplemented!()`
- No `print!`/`println!` — use `tracing` macros
- All items must have documentation comments
- All errors map to `McpError::internal_error()` or `McpError::invalid_params()`
- Use `schemars::JsonSchema` for all parameter/response structs

## Environment Variables

- `ZENMONEY_TOKEN` — Required API access token
