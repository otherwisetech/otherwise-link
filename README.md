# otherwise-link

The shared integration layer of [Otherwise Technologies](https://otherwisetech.com)' desktop
apps: deep links between them, finding which ones are installed, one registry of the people,
companies, vehicles, properties, devices and sites they all talk about, a Today feed that
gathers what needs attention, a calendar feed (with ICS export / import and computed public
holidays), and one MCP server for all of them.

Pure Rust with no UI dependency, so any app (open or closed source, GPUI or not) can use it.
macOS, Windows and Linux.

| Crate | What |
|-------|------|
| [`otherwise-link`](crates/link) | Library: `link`, `open`, `register`, `apps`, `entity`, `tax`, `registry`, `today`, `events`, `ics`, `holidays`, `settings`, `paths`, `background` |
| [`otherwise-mcp-hub`](crates/hub) | Library + `otherwise-hub` binary: a stdio MCP server proxying every installed app's MCP tools (`expense.*`, `vigia.*`, `nodo.*`) plus `entities_*`, `today`, `calendar_events` and `open_link` |

- **Spec**: [docs/SPEC.md](docs/SPEC.md) (link syntax, card JSON, `entities.db`, Today files,
  `hub.json`, discovery, MCP naming).
- **Integrating an app**: [docs/INTEGRATING.md](docs/INTEGRATING.md).

```toml
[dependencies]
otherwise-link = { git = "https://github.com/otherwisetech/otherwise-link", tag = "v0.5.2" }
```

```rust
use otherwise_link::{Link, Root, today::{self, Event, EventKind, Severity}};

// a deep link into Vigía
let link = Link::new("vigia").segment("item").segment("42");
assert_eq!(link.to_url(), "otherwise://vigia/item/42");

// tell the Today view about a deadline
today::publish(&Root::system(), "vigia", &[
    Event::new("item-42-due", EventKind::Due, "SOAT ABC123").when("2026-11-30").severity(Severity::High).link(&link),
])?;
```

## Privacy and safety

- Everything stays in one folder the user picks, the data root (default `~/.otherwise`; it can
  be moved to OneDrive, Dropbox, iCloud Drive… from Otherwise Apps). Moves are copy-verify-then-
  delete; files open on another computer are protected by a lock. That is not multi-device sync.
- The entity registry is **opt-in**: nothing creates it until the user turns it on in
  Otherwise Apps.
- `otherwise-hub` is **read-only** unless the user enables write tools in Otherwise Apps, and
  that switch is checked on every call. It never edits your MCP client's configuration.

## MCP

```
claude mcp add otherwise-hub -- "/Applications/Otherwise Apps.app/Contents/MacOS/otherwise-apps" mcp
# or with the standalone binary
cargo install --git https://github.com/otherwisetech/otherwise-link otherwise-mcp-hub
claude mcp add otherwise-hub -- otherwise-hub
otherwise-hub list      # which apps were found and how many tools each offers
```

## Develop

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## License

MIT OR Apache-2.0, © Otherwise Technologies LLC.
