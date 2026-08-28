# Upstream GRACE source

Worldline vendors the upstream GRACE workflow skills as its repository-local
engineering control layer.

## Source

- Repository: https://github.com/osovv/grace-marketplace
- Copied commit: `4f81342993dc08c4f701be51ea51ae291a53a778`
- Upstream tag: `v4.0.4`
- Worldline adoption date: `2026-08-28`
- Local path: `.agents/skills/grace/`
- Local CLI verified during adoption: `grace 4.0.4`

The files were copied from the already-vendored, provenance-recorded
SignalWeave snapshot. No Worldline behavior is embedded in the vendored skill
sources.

## Local adaptation

Worldline-specific workflow rules live in `AGENTS.md` and `docs/grace/`.
Durable project state lives only under `.grace/`. The optional
`@osovv/grace-cli` package is not a Cargo or runtime dependency; the CLI is an
engineering tool.
