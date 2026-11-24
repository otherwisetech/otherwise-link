# otherwise-link — for agents and contributors

The shared integration layer of Otherwise Technologies' desktop apps (genealogy-rs, diskmap,
rama, mosaico, expense, Vigía, Nodo, Otherwise Hub…). Pure Rust, **no GPUI and no UI**, so
open-source and closed-source apps can both depend on it. Apps depend on it by git tag:

```toml
otherwise-link = { git = "https://github.com/otherwisetech/otherwise-link", tag = "v0.5.4" }
```

## Layout

```
crates/link  otherwise-link (lib `otherwise_link`):
             link.rs (otherwise:// parse / build), open.rs (open_link, system opener),
             register.rs (CFBundleURLTypes / .desktop MimeType / HKCU registry snippets + runners),
             apps.rs (KNOWN app table, otherwise-app.json self-descriptions, Locations, detect / discover, Info.plist + install marker),
             entity.rs (cards: kinds, normalize, natural keys, absorb / apply), tax.rs (tax ids +
             check digits), registry.rs (SQLite entities.db: migrations, CRUD, merge, change log,
             cursors), today.rs (per-app feed files, merge, expiry, dismissals), events.rs
             (<root>/<app>/events.json calendar feed, RRULE subset, expansion, merge), ics.rs
             (iCalendar write / parse), holidays.rs (computed public holidays, business days), settings.rs
             (hub.json), paths.rs (data root: ~/.otherwise/config.json pointer, app_dir), relocate.rs
             (plan / move / copy the root, pre-0.2 migration + .migrated marker), storage.rs
             (verified SQLite / file / folder copy + move, rollback journal, status), cloud.rs
             (cloud-folder detection, conflicted copies), lock.rs (<file>.lock.json: host / pid
             / heartbeat, stale), fsutil.rs (atomic writes), text.rs (accent folding),
             background.rs (otherwise-background.json in the install, scan / verify-before-run
             plan, job reports, notification outbox, per-app lease so one computer runs a
             root's jobs); paths.rs also has device_dir() (this computer's state, never synced)
crates/hub   otherwise-mcp-hub (lib `otherwise_mcp_hub` + bin `otherwise-hub`): child.rs (one
             app's MCP server as a child process), hub.rs (config from discovery, routing,
             read-only gate, restart), tools.rs (entities_* / today / calendar_events / open_link), rpc.rs
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
- **Never lose the user's data.** Moves are copy → verify → switch → delete the original; never
  overwrite at a destination; tests cover every move path (storage.rs, relocate.rs). The
  pointer file `~/.otherwise/config.json` is the only fixed path; everything else follows
  `data_root()`, read fresh each time.
- **Device state never lives in the data root** (`paths::device_dir()`, not the pre-0.2 folder);
  `relocate` never carries it, nor background leases.
- **Never write another app's Today or events file**; writes are atomic (`fsutil::write_atomic`).
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
