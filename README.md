# Rove

Rove is a local-first coding agent: pick a workspace, hand it a task, watch it execute in real time, approve sensitive operations, then review files, diffs, artifacts, and usage. Desktop, Web, CLI/TUI, HTTP API, and benchmarks all run on the same persistent runtime, and interrupted sessions resume after a disconnect or restart. This is a 0.1.0 source prerelease, suitable for local development and internal trial.

## Quick start

Requires Git and Rust stable (see `rust-toolchain.toml`); Web/Desktop also need Node 22 and pnpm 10.

```bash
git clone https://github.com/AllureCurtain/rove.git && cd rove
cargo run -p rove-cli -- --model fake                     # TUI with no API key

cd apps/web && pnpm install --frozen-lockfile && cd ../..
powershell -ExecutionPolicy Bypass -File scripts/dev.ps1  # API + Web, fake provider
```

Then open <http://localhost:3000>. The API is at <http://127.0.0.1:8787>, with API docs at `/swagger-ui`.

For real model providers, installing the `rove` command, and running Desktop, see docs/user-guide.md.

## Documentation

- User guide: docs/user-guide.md
- Development guide: docs/development.md
- Architecture: docs/architecture.md

## License

MIT, see [LICENSE](LICENSE).
