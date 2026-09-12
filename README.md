<div align="center">
  <img src="public/kiwano-logo.svg" width="112" alt="Kiwano logo" />
  <h1>Kiwano</h1>
  <p><strong>Local-first AI provider manager.</strong><br/>
  Keep every provider key on your own machine, point all your coding agents at one local gateway, and see what they actually cost.</p>
  <p>
    <a href="https://github.com/lightconsen/kiwano/releases/latest"><img src="https://img.shields.io/github/v/release/lightconsen/kiwano?label=download&sort=semver" alt="Latest release"></a>
    <a href="https://github.com/lightconsen/kiwano/actions/workflows/ci.yml"><img src="https://github.com/lightconsen/kiwano/actions/workflows/ci.yml/badge.svg" alt="CI status"></a>
    <img src="https://img.shields.io/badge/license-GPLv3-blue" alt="GPLv3 license">
  </p>
</div>

Kiwano is a desktop app that sits between your coding agents and your AI providers. You register providers once, it runs a local gateway on `127.0.0.1:8317`, and every agent — Claude Code, Codex, Gemini CLI and others — talks to that one port. The gateway normalizes protocols, routes each request through the strategy you picked, meters what it cost, and records what happened.

Your keys stay on your machine, in an owner-only local database. Requests never touch a Kiwano server.

![Kiwano's Apps screen: providers with their bound agents, usage and quota for the day](docs/screenshots/apps.png)

> Captured from the development build with its bundled sample data set.

## Features

- **Provider management** — add, edit, switch and delete provider configs. Route with `single`, `failover`, `roundrobin`, `timewindow` or `quota` strategies, each with its own candidate weights and windows.
- **Agent takeover** — connect Claude Code, Codex, Gemini CLI, Grok Build, Claude Desktop, OpenCode, OpenClaw, Hermes and Pi to the local gateway in one click. Each agent's original config is backed up and restored when you switch it off.
- **Local gateway** — one always-on port with protocol normalization (anthropic / openai / gemini), so a provider that speaks one dialect can serve an agent that speaks another. Hot-reloads on change; the daemon outlives the GUI.
- **Usage and cost** — 7-day trends for requests, tokens, cost and latency, attributed per provider and per agent, with quota rings for metered plans and per-period cost alerts.
- **Models shelf** — a built-in Hub catalog of 82 providers (official / aggregator / third-party / free), with live search and one-click add. The catalog syncs conditionally: a manifest hash skips the download when nothing changed, and the bundled copy keeps it working offline.
- **Request logs** — every gateway request with status, latency and token accounting; filter down to errors.
- **cc-switch import** — read an existing cc-switch configuration and migrate it in.
- **Self-update** — signed releases (minisign); the app checks at startup and updates in place. 0.1.2 and later update themselves to newer versions.

## Download

Latest release: **https://github.com/lightconsen/kiwano/releases/latest**

| Platform | File |
| --- | --- |
| macOS — Apple Silicon (M series) | [`Kiwano_aarch64.dmg`](https://hub.kiwano.cc/releases/Kiwano_aarch64.dmg) |
| macOS — Intel | [`Kiwano_x64.dmg`](https://hub.kiwano.cc/releases/Kiwano_x64.dmg) |
| Windows — installer | [`Kiwano_x64-setup.exe`](https://hub.kiwano.cc/releases/Kiwano_x64-setup.exe) |
| Windows — MSI | [`Kiwano_x64_en-US.msi`](https://hub.kiwano.cc/releases/Kiwano_x64_en-US.msi) |
| Linux — AppImage | [`Kiwano_amd64.AppImage`](https://hub.kiwano.cc/releases/Kiwano_amd64.AppImage) |
| Linux — Debian / Ubuntu | [`Kiwano_amd64.deb`](https://hub.kiwano.cc/releases/Kiwano_amd64.deb) |
| Linux — Fedora / RHEL | [`Kiwano-1.x86_64.rpm`](https://hub.kiwano.cc/releases/Kiwano-1.x86_64.rpm) |

Those links are served from Cloudflare R2 and always point at the newest build — no version in the URL, so they stay valid across releases. The same files are attached to every [GitHub release](https://github.com/lightconsen/kiwano/releases) as a mirror.

**macOS builds are signed with a Developer ID and notarized**, so the app opens the usual way. Pick the build for your chip: `aarch64` is Apple Silicon, `x64` is Intel (it runs under Rosetta, but the updater expects the build matching your chip).

**Windows builds are not code-signed yet.** SmartScreen will warn on first launch; choose **More info → Run anyway**.

**Linux** — `chmod +x` the AppImage, or install the `.deb` / `.rpm` with your package manager.

### Verify your download

Every release attaches `SHA256SUMS`, and every artifact carries a signed build-provenance attestation.

**Provenance** — proves the file was built by this repository's release workflow at the tagged commit, not merely that it is intact. It is checked against GitHub's transparency log, so it does not depend on trusting the host you downloaded from:

```sh
gh attestation verify Kiwano_x64.dmg --repo lightconsen/kiwano
```

**Checksum** — compare the digest in `SHA256SUMS` (attached to the [GitHub release](https://github.com/lightconsen/kiwano/releases)) with the file you have:

```sh
# the published digest
grep ' Kiwano_x64.dmg$' SHA256SUMS
# the digest of the file you have
sha256sum Kiwano_x64.dmg                    # Linux
shasum -a 256 Kiwano_x64.dmg                # macOS

# Windows, for Kiwano_x64-setup.exe
certutil -hashfile Kiwano_x64-setup.exe SHA256
```

The two lines must match. Note that `SHA256SUMS` records the GitHub release names, which carry the version; the R2 links above deliberately have none, so pair it with the matching GitHub release if you downloaded from R2. Provenance is the check that needs no name to line up.

Updates are delivered as `*.app.tar.gz` / `*-setup.exe` / `AppImage` artifacts from the release manifest (`latest.json`), signature-checked against the public key compiled into the app.

## Privacy

Kiwano is local-first by design, and the code is the specification:

- API keys are stored **locally, in an owner-only SQLite database** — directory `0700`, file `0600` — so only your user can read them. Provider credentials are only ever sent to the provider you configured.
- Requests, prompts and responses **never pass through a Kiwano server** — the gateway runs on your machine.
- Usage history lives in a **local SQLite database** (`~/.kiwano/kiwano.db`). Nothing is uploaded.
- The Hub only ever serves **catalog metadata** (provider names, endpoints, prices). It sees no keys and no request data.
- There is no analytics or telemetry in the app. Update checks are a plain HTTPS request for a static JSON manifest.

## Requirements

- macOS (Apple Silicon or Intel), Windows 10 or later, or Linux x86_64
- Rust **1.98.0** to build — pinned by `rust-toolchain.toml`, so `rustup` picks it up automatically
- Node 24 and pnpm 10 for the frontend

## Development

```bash
pnpm install
pnpm dev         # frontend dev server on :1420 (browser, mocked data)
pnpm tauri dev   # desktop app in dev mode (real SQLite + gateway sidecar)

pnpm build       # frontend typecheck + production build
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

The app does not contain the gateway; it spawns it as a sidecar daemon, and the
two are built and shipped together. `pnpm tauri` builds the gateway for the host
before handing over to the CLI, so the first `pnpm tauri dev` on a clean tree
compiles it before the frontend starts. For a cross build, name the target:
`KIWANO_SIDECAR_TARGET=x86_64-apple-darwin pnpm tauri build`. At runtime
`KIWANO_GATEWAY_BIN` overrides which binary the app looks for.

**Run `pnpm build:sidecar` first on a fresh clone.** The gateway is declared as
an `externalBin`, and Tauri resolves that in `src-tauri/build.rs` — so a missing
sidecar fails `cargo clippy` and `cargo test` themselves, not just the bundle
step. `pnpm tauri dev` does it for you; the bare cargo commands do not.

CI runs exactly those four Rust checks on Linux and macOS, plus a frontend typecheck and build. The toolchain version is pinned in `rust-toolchain.toml` and in every workflow, so a new clippy release can't turn the build red on code nobody touched.

Layout:

| Path | What lives there |
| --- | --- |
| `src/` | React UI (screens, components, the `api/` layer) |
| `src-tauri/` | Tauri app: commands, view models, updater, agent discovery; the bundled provider catalog is `src-tauri/src/catalog.json` |
| `crates/gateway/` | The gateway daemon: routing, strategies, metering, store |
| `crates/adapters/` | Protocol conversion, agent config take-over, pricing table (`resources/models.json`) |
| `crates/cli/` | `kiwano` command-line client |
| `site/`, `design/` | Marketing site, and the original UI design prototype |

## License

GNU General Public License v3.0 or later, see [LICENSE](LICENSE).

Selected modules are ported from [cc-switch](https://github.com/farion1231/cc-switch) (MIT); ported files carry upstream declarations, see [THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES).
