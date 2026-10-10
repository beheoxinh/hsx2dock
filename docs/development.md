# Development

## Prerequisites (Nobara / GNOME)

- GTK 4.12+, libadwaita 1.4+
- Docker Engine or Docker Desktop
- `systemctl --user` for daemon Start/Stop

```sh
sudo dnf install gtk4-devel libadwaita-devel pkg-config desktop-file-utils
```

## Build & run

```sh
cargo build
./target/debug/docker-manager

# release
cargo build --release
```

## Checks (mirror CI)

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --lib
cargo test --test integration            # 2 pass, 1 ignored (needs daemon)
cargo doc --no-deps --document-private-items   # with RUSTDOCFLAGS=-D warnings
desktop-file-validate data/*.desktop
```

## Tests

- Unit tests live next to the code (`#[cfg(test)] mod tests`):
  `format_bytes` / `format_relative*` (`ui/common.rs`),
  `RunConfig::validate` / `parse_lines` (`docker/models.rs`),
  port formatting + run validation (`docker/client.rs`).
- Gated integration tests live in `tests/integration.rs`.
  Daemon-dependent cases are `#[ignore]`d; run them against a live
  daemon with `DOCKER_TEST=1 cargo test -- --include-ignored`.

## Desktop integration

```sh
sudo install -Dm644 data/com.github.docker_manager.desktop \
  /usr/share/applications/com.github.docker_manager.desktop
sudo install -Dm644 data/icons/com.github.docker_manager.svg \
  /usr/share/icons/hicolor/scalable/apps/com.github.docker_manager.svg
```

## Release checklist

1. Bump `version` in `Cargo.toml`.
2. Update `README.md` feature list + architecture tree.
3. `cargo doc` clean, `desktop-file-validate` passes.
4. Tag and build release.
