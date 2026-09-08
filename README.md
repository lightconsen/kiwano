<div align="center">
  <img src="public/kiwano-logo.svg" width="112" alt="Kiwano logo" />
  <h1>Kiwano</h1>
  <p>Local-first AI provider manager — manage all your AI providers on one machine and route coding agents through a local gateway.</p>
</div>

## Features

- **Provider management** — add, switch and delete provider configs; gateway routing with failover / roundrobin / timewindow / quota strategies.
- **Agent takeover** — one-click connect Claude Code, Codex, Gemini CLI and more to the local gateway (`127.0.0.1:8317`); disabling restores each agent's original configuration.
- **Local gateway** — a single always-on port with protocol normalization (anthropic / openai / gemini); every request is routed and metered.
- **Usage dashboard** — 7-day trends of requests, tokens, cost and latency, attributed per provider and agent.
- **Provider shelf** — built-in Hub catalog (177 providers, filterable by official / aggregator / third-party / free), one-click add.
- **cc-switch import** — read existing cc-switch configs and migrate seamlessly.

## Development

```bash
pnpm install
pnpm dev         # frontend dev server
pnpm tauri dev   # desktop app in dev mode
pnpm build       # frontend production build
```

## License

Apache License 2.0, see [LICENSE](LICENSE).

Selected modules are ported from [cc-switch](https://github.com/farion1231/cc-switch) (MIT); ported files carry upstream declarations, see [THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES).
