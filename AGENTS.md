# Coding agent guidelines

This repository is **tnpm**, a Rust 2024 edition package manager that prioritizes speed and a documented subset of npm compatibility. Read [README.md](README.md) for user-facing behavior and [docs/README.md](docs/README.md) for detailed specifications.

## Layout and ownership

- `src/cli.rs` and `src/commands/`: CLI parsing and commands.
- `src/install/`, `src/lockfile.rs`, `src/lockfile/`: dependency resolution, layout, package installation, and lockfiles.
- `src/registry/` and `src/config/`: HTTP transport, caching, registry configuration, and npmrc.
- `src/runner/` and `src/scripts.rs`: JS execution and package lifecycle scripts.
- `tests/commands/`: CLI integration tests; `tests/support/`: shared isolated fixtures.
- `benchmarks/`: benchmark programs and recorded results; keep benchmark history separate from tests.

See [docs/architecture.md](docs/architecture.md) for more precise responsibility boundaries.

## Change guidelines

- Keep the root README focused on installation and quickstart. Put extensive command descriptions in [docs/commands.md](docs/commands.md), compatibility details in [docs/compatibility.md](docs/compatibility.md), and implementation details in the matching docs page.
- Document intentional behavior changes alongside the code; do not claim full npm compatibility or unsupported features.
- Preserve existing lockfile and cache safety checks (integrity, path traversal, symlinks, and atomic replacement). Prefer tests around externally observable behavior over implementation-only tests.
- New CLI integration test modules belong in `tests/commands/` and must be registered in `tests/commands/mod.rs`.
- Tests should use local loopback HTTP fixtures, not live npm or GitHub endpoints. npm interoperability tests are opt-in/ignored; keep them separate from the default suite.
- For installer changes, maintain checksums before replacement and do not require admin rights. The Windows installer updates the **user** PATH only. Keep Unix installers compatible with POSIX `sh`.
- Avoid rewriting existing benchmark results unless the task explicitly involves new measurements.

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
sh -n install.sh
```

For benchmark tooling, see [docs/development.md](docs/development.md). Release asset names and supported targets are defined in [`.github/workflows/release.yml`](.github/workflows/release.yml); any installer changes must stay in sync with them. Tag-based releases require a `v<version>` tag matching `Cargo.toml`.
