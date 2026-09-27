# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Bootstrap crate scaffold: package metadata, dual MIT or Apache-2.0 license, and a committed `Cargo.lock` for reproducible builds.
- CI on GitHub Actions: rustfmt, clippy, rustdoc, and tests on a pinned toolchain, plus a guard against references to private repositories.
- `AGENTS.md` describing scope, core invariants, and dependency policy.
