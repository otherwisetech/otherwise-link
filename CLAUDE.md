# otherwise-link — for agents and contributors

The shared integration layer of Otherwise Technologies' desktop apps (genealogy-rs, diskmap,
rama, mosaico, expense, Vigía, Nodo, Otherwise Apps…). Pure Rust, **no GPUI and no UI**, so
open-source and closed-source apps can both depend on it. Apps depend on it by git tag:

```toml
otherwise-link = { git = "https://github.com/otherwisetech/otherwise-link", tag = "v0.1.0" }
```

## Layout

```
crates/link  otherwise-link (lib `otherwise_link`):
             link.rs (otherwise:// parse / build), open.rs (open_link, system opener),
             register.rs (CFBundleURLTypes / .desktop MimeType / HKCU registry snippets + runners),
             apps.rs (KNOWN app table, Locations, detect / discover, Info.plist + install marker),
             entity.rs (cards: kinds, normalize, natural keys, absorb / apply), tax.rs (tax ids +
             check digits), registry.rs (SQLite entities.db: migrations, CRUD, merge, change log,
             cursors), today.rs (per-app feed files, merge, expiry, dismissals), settings.rs
             (hub.json), paths.rs (the shared Otherwise folder), fsutil.rs (atomic writes),
             text.rs (accent folding)
crates/hub   otherwise-mcp-hub (lib `otherwise_mcp_hub` + bin `otherwise-hub`): child.rs (one
             app's MCP server as a child process), hub.rs (config from discovery, routing,
             read-only gate, restart), tools.rs (entities_* / today / open_link), rpc.rs
docs/        SPEC.md (formats: normative), INTEGRATING.md (how an app plugs in)
```

## Rules

- **Formats are a contract between separately released apps.** Anything in docs/SPEC.md
  (link syntax, card JSON, entities.db schema, Today files, hub.json, tool names) changes only
  compatibly: add optional fields, never rename / remove / change meaning. A breaking change
  bumps the schema number, readers refuse newer schemas, and SPEC.md says so.
- **entities.db**: schema changes are a new entry at the end of `registry::MIGRATIONS` (never
  edit a shipped one) plus a test. Rollback journal (`journal_mode=DELETE`, never WAL), 10 s
  busy timeout, every write one `BEGIN IMMEDIATE` transaction, never hold one across calls.
  Nothing creates the file before the user opted in (`Registry::open_shared`).
- **Never write another app's Today file**; writes are atomic (`fsutil::write_atomic`).
- **The hub is read-only by default.** Write tools (own, and child tools not annotated
  `readOnlyHint: true`) only with `hub.json` `mcpWrites`, re-checked on every call. Never touch
  the user's Claude / Codex / MCP client config: we print the `claude mcp add` command, the
  user runs it.
- No user-facing UI text lives here (apps translate); error and tool descriptions are English.
- Tests never touch the real shared folder, /Applications, the registry of the OS or the
  network: everything takes a `Root`, `Locations` or an opener. Every change ships with a test.
- Comments: sparse, only non-obvious why. Same rusqlite / chrono / uuid versions as vigia-rs,
  nodo-rs and expense-rs (two libsqlite3-sys in one build don't link).

## Validate

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Release: bump `workspace.package.version`, tag `vX.Y.Z` on main once CI is green.
