# Contributing to Kiwano

Thanks for considering it. This is a small project, so the most valuable
contributions are **bug reports with a reproduction** and **small, focused pull
requests**.

## Before you write code

- Anything bigger than a bug fix — a new agent adapter, a new routing strategy,
  a UI rework — should start as an issue. It saves you from writing something
  that then has to be reshaped to fit.
- Questions and "how do I …" go to [Discussions](https://github.com/lightconsen/kiwano/discussions),
  not issues.
- Security issues: never in public. See [SECURITY.md](SECURITY.md).

## Development setup

The frontend and the Tauri shell live under `app/`; the Rust workspace root is
the repo root, because `crates/` is part of it.

```bash
cd app
pnpm install
pnpm build:sidecar     # required on a fresh clone — see the gotcha below
pnpm dev               # frontend dev server on :1420, mocked data
pnpm tauri dev         # desktop app, real SQLite + gateway sidecar
```

Requirements: Rust **1.98.0** (pinned by `rust-toolchain.toml`, so `rustup`
picks it up), Node 24 and pnpm 10.

`pnpm dev` runs the UI in a browser against the bundled sample data set — no
gateway, no real keys. That is the fastest loop for UI work, and it is what the
screenshots in `docs/screenshots/` were captured from.

## The four checks CI runs

Run all of these before pushing; CI runs exactly these and nothing else:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cd app && pnpm build && pnpm test
```

Two gotchas that cost time if you haven't hit them before:

1. **The sidecar.** The gateway is declared as an `externalBin` and Tauri
   resolves it in `app/src-tauri/build.rs`, so a missing sidecar fails
   `cargo clippy` and `cargo test` themselves — not just the bundle step.
   `pnpm tauri dev` stages it for you; the bare cargo commands do not.
2. **No Windows-reserved filenames.** `aux.rs`, `con.rs`, `com1.rs`, `nul.rs`
   and friends cannot be checked out on Windows at all, which makes the
   repository un-clonable there. CI rejects them on the fast Linux job, because
   the failure it otherwise produces looks like an infrastructure problem.

## Pull requests

- One logical change per PR.
- Visible in the UI? Say so, and include a screenshot.
- Anything user-visible needs a `CHANGELOG.md` entry: add it under
  `[Unreleased]`. The release workflow copies that section into the GitHub
  release body, so it is what people read before they download.
- New UI strings go through the i18n dictionaries in `app/src/i18n/` — every
  language needs the key, and there is a test that enforces it.
- Comment the *why*, not the *what*. The codebase already leans that way;
  several files carry long comments about a mistake that was made once. Please
  keep it that way.

## Reporting bugs

Use the bug template. The single most useful extra thing you can include is
whether it still reproduces with a freshly added provider — that separates "my
old config" from a real defect.

## License

By contributing you agree that your work is licensed under GPLv3-or-later, the
same as the project.
