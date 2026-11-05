# wapit

A local WhatsApp Cloud API mock and inbox, shipped as one Rust binary. README.md describes what it does for users; this file is for whoever changes it.

## Layout

- `src/http/graph.rs`: the Graph API mock on port 8027. Routing is a single fallback handler because Meta's paths are ids, not fixed routes.
- `src/http/ui.rs`: the inbox API and static files on port 8026. `public/` is embedded in the binary with `rust-embed`.
- `src/http/mod.rs`: the `Host` allowlist both servers sit behind.
- `src/validate.rs`: outbound payload validation and template rendering. Each rule mirrors a documented Meta error code; keep the code and message text identical to Meta's.
- `src/templates.rs`: template create and edit through the Graph API.
- `src/inbox.rs`: the service the handlers call: sends, statuses, user actions, media, webhooks, change events.
- `src/webhooks.rs`: payload envelopes, `X-Hub-Signature-256` signing and delivery.
- `src/store.rs`: SQLite. Schema changes are new entries in `MIGRATIONS`; never edit a shipped migration.
- `src/config.rs`: `apps.json` and `templates/*.json`, reloaded on change, last good version kept on errors.
- `public/`: the inbox UI in plain HTML, CSS and JavaScript, with no build step. The visual design lives in the Paper file `wapit`; change the design there first.
- `tests/api.rs`: the HTTP contract, end to end, against real listeners.

## Rules

- `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` must pass. Clippy runs with `pedantic`.
- wapit only talks to Meta in `src/sync.rs`, which reads templates when the user runs `wapit sync`. Nothing else may reach `graph.facebook.com`.
- A behavior change that touches Meta's contract (payload shapes, error codes, webhook bodies) needs a test in `tests/api.rs` and a source in Meta's documentation.
- Server text that reaches the inbox is English. The UI owns its wording and translations in `public/i18n.js`; the server sends codes, such as `Issue`, not sentences.
- Add a dependency only when the standard library or an existing crate cannot do the job, and keep `Cargo.lock` committed. `cargo deny check` runs in CI.
- Issues are the only way in for outside contributions: the repository only lets collaborators open pull requests, and changes are made by the maintainer. See CONTRIBUTING.md.
