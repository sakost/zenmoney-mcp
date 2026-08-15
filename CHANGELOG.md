# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.1](https://github.com/sakost/zenmoney-mcp/compare/v0.4.0...v0.4.1) - 2026-08-15

### Fixed

- Initial sync no longer crashes on system merchants with
  `"user": null` — the root cause of
  [#7](https://github.com/sakost/zenmoney-mcp/issues/7), fixed by
  @samosvat in zenmoney-rs 0.4.1
  ([sakost/zenmoney-rs#4](https://github.com/sakost/zenmoney-rs/pull/4))

### Added

- Prebuilt binaries for Linux (x86_64/aarch64), macOS
  (Intel/Apple Silicon), and Windows are now built and attached to
  every release automatically
  ([#5](https://github.com/sakost/zenmoney-mcp/issues/5));
  `cargo binstall zenmoney-mcp` is supported

## [0.4.0](https://github.com/sakost/zenmoney-mcp/compare/v0.3.1...v0.4.0) - 2026-08-02

### Fixed

- Startup sync no longer crashes on accounts with `day`/`week` payoff
  intervals ([#10](https://github.com/sakost/zenmoney-mcp/issues/10))
- Sync deserialization errors now report the exact JSON field path
  (e.g. `account[0].title`) instead of a byte offset, via
  zenmoney-rs 0.4.0 ([#7](https://github.com/sakost/zenmoney-mcp/issues/7))

### Changed

- Upgraded `rmcp` 0.17 → 3.1, resolving RUSTSEC-2026-0189
  ([#12](https://github.com/sakost/zenmoney-mcp/issues/12))
- Updated `zenmoney-rs` to 0.4.0
