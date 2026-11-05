# wapit

A local WhatsApp Cloud API mock and inbox, shipped as one Rust binary. README.md describes what it does for users; this file is for whoever changes it.

## Layout

- `src/http/graph.rs`: the Graph API mock on port 8027. Routing is a single fallback handler because Meta's paths are ids, not fixed routes.
- `src/http/ui.rs`: the inbox API and static files on port 8026. `public/` is embedded in the binary with `rust-embed`.
- `src/http/mod.rs`: the `Host` allowlist both servers sit behind.
- `src/validate.rs`: outbound payload validation and template rendering. Each rule mirrors a documented Meta error code; keep the code and message text identical to Meta's.
- `src/templates.rs`: the one owner of an app's templates: list, create, edit, delete and the `wapit sync` replace, over `ConfigStore`.
- `src/inbox.rs`: the service the handlers call: sends, statuses, user actions, media, webhooks, templates and the read views. Its store and config are private, and only it emits change events.
- `src/views.rs`: the typed read views and change events the inbox page reads.
- `src/webhooks.rs`: payload envelopes, `X-Hub-Signature-256` signing and delivery.
- `src/store.rs`: SQLite. Schema changes are new entries in `MIGRATIONS`; never edit a shipped migration.
- `src/config.rs`: `apps.json` and `templates/*.json`, reloaded on change, last good version kept on errors.
- `public/`: the inbox UI in plain HTML, CSS and JavaScript, with no build step. The visual design lives in the Paper file `wapit`; change the design there first.
- `tests/api.rs`: the HTTP contract, end to end, against real listeners.
- `site/`: the public site at https://wapit.patriciomartins.com.br, an assets-only Cloudflare Worker with no Access in front. The landing pages are hand-written HTML in `site/public/` (`index.html` and `pt-br/index.html`). The docs come from `site/content/docs.en.md` and `docs.pt-br.md`: `node site/build.mjs` renders them into `public/docs/` and `public/pt-br/docs/` and publishes the same Markdown as `llms-full.txt`, so the page and its "Copy as Markdown" buttons never disagree. Those outputs are git-ignored; `wrangler deploy` runs the build. The `Site` workflow deploys on pushes to `main` that touch `site/`, through the `site` environment and its `CLOUDFLARE_API_TOKEN` secret. Its design lives on the Site page of the Paper file.

## Rules

- `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` must pass. Clippy runs with `pedantic`.
- wapit only talks to Meta in `src/sync.rs`, which reads templates when the user runs `wapit sync`. Nothing else may reach `graph.facebook.com`.
- A behavior change that touches Meta's contract (payload shapes, error codes, webhook bodies) needs a test in `tests/api.rs` and a source in Meta's documentation.
- Server text that reaches the inbox is English. The UI owns its wording and translations in `public/i18n.js`; the server sends codes, such as `Issue`, not sentences.
- A user-facing change updates the README and both docs sources in `site/content/` in the same change.
- Add a dependency only when the standard library or an existing crate cannot do the job, and keep `Cargo.lock` committed. `cargo deny check` runs in CI.
- Issues are the only way in for outside contributions: the repository only lets collaborators open pull requests, and changes are made by the maintainer. See CONTRIBUTING.md.
