# Integrating an app with otherwise-link

For whoever wires Vigía, Nodo, genealogy-rs, mosaico, diskmap, rama (or a new app) into the
shared layer. The formats are in [SPEC.md](SPEC.md); this is the how-to. Every step is
optional and independent: do the ones that make sense for the app.

```toml
# the crate that has the logic (core / cli), not only the GPUI app
otherwise-link = { git = "https://github.com/otherwisetech/otherwise-link", tag = "v0.2.0" }
```

Rules that carry over from the apps' CLAUDE.md: shared features are **opt-in or read-only by
default**, nothing leaves the computer, tests use temp dirs (`Root::at(tmp)`,
`Locations { … }`), never the real shared folder. Keep rusqlite at 0.37 (same
`libsqlite3-sys` as otherwise-link) or the build won't link. Add the app's id to
`apps::KNOWN` in otherwise-link if it isn't there (and tag a new version).

## 1. Deep links: register `otherwise-<id>://` and open links

1. **macOS bundle script** (`script/bundle-macos.sh`): paste into the `Info.plist` `<dict>` the
   output of `otherwise_link::register::macos_url_types("<bundle id>", &["otherwise-<id>"])`,
   i.e.:

   ```xml
   <key>CFBundleURLTypes</key>
   <array><dict>
     <key>CFBundleURLName</key><string>io.github.otherwisetech.vigia-app</string>
     <key>CFBundleTypeRole</key><string>Viewer</string>
     <key>CFBundleURLSchemes</key><array><string>otherwise-vigia</string></array>
   </dict></array>
   ```

   The script's `lsregister -f` (or copying to /Applications) registers it.
2. **Linux**: in `packaging/linux/<app>.desktop` add `MimeType=x-scheme-handler/otherwise-<id>;`
   and make `Exec=` end in `%u` (`register::linux_desktop_patch` does both). When the app
   installs its menu entry (or Otherwise Apps does), call
   `register::register_linux("<bundle id>.desktop", &["otherwise-<id>"])`.
3. **Windows**: on first start (cheap, idempotent), call
   `register::register_windows("otherwise-<id>", &std::env::current_exe()?, "<Name>")`
   (per-user `HKCU`, no admin).
4. **Receive links.**
   - macOS (GPUI): before `run`, `application.on_open_urls(|urls| …)`; push parsed links into a
     static queue and drain it from the app (same pattern as expense's `INBOUND`), because the
     callback has no `App` context and can fire before the window exists.
   - Linux / Windows: `Link::from_args(std::env::args())` at start. If the app is
     single-instance, forward it to the running instance.
   - Handle at least `entity/<uuid>` (look the card up with `Registry::get`, use
     `refs["<id>"]` to find your record) and the empty path (focus). Document your own paths
     (`item/<id>`, `device/<id>`…) in your README.
5. **Open other apps' links** with `otherwise_link::open::open_link(&link)` (tries
   `otherwise-<app>://`, falls back to `otherwise://`, which Otherwise Apps handles). Build
   links with `Link::new("nodo").segment("device").segment(id)`.

## 2. Publish Today events

One file per app; rewrite it whole whenever the list changes (start-up, after an edit, on the
app's timer). Cheap and atomic; never write another app's file.

```rust
use otherwise_link::{Link, Root, today::{self, Event, EventKind, Severity}};

let events: Vec<Event> = due_items.iter().map(|it| {
    Event::new(format!("item-{}-due", it.id), EventKind::Due, it.title.clone())   // stable id
        .when(it.deadline.to_string())                     // "YYYY-MM-DD" or RFC 3339
        .severity(if it.days_left <= 3 { Severity::High } else { Severity::Normal })
        .link(&Link::new("vigia").segment("item").segment(it.id.to_string()))
        .expires((it.deadline + Duration::days(7)).to_string())
}).collect();
today::publish(&Root::system(), "vigia", &events)?;   // [] when nothing is pending
```

- Keep ids stable (the user's dismissals key on `app/id`); a new cycle (renewed item) may take
  a new id so it shows again.
- Titles are already-translated text in the user's language (the app's current locale); keep
  them short (≤ 200 chars). Put detail in `body`.
- Kinds: `due` (deadline), `alert` (something wrong now: device down, cert expired), `review`
  (inbox to review), `info`. Don't flood: at most 200 events, prefer the next 30 days.
- If the user turns the feature off in your app, call `today::clear(&root, "<id>")`.
- Add a setting ("Show in Otherwise Apps' Today") if the content is sensitive; default on is
  fine for plain reminders.

## 3. Sync entities

Only when the user turned the shared registry on (`Registry::open_shared` returns
`Error::Disabled` otherwise; treat that as "feature off", silently).

```rust
use otherwise_link::{Root, registry::{Registry, Error, Op}, entity::{Entity, Kind, Company}, tax::TaxId};

let root = Root::system();
let mut reg = match Registry::open_shared(&root) { Ok(r) => r, Err(Error::Disabled) => return Ok(()), Err(e) => return Err(e.into()) };

// push: your record → a card with your ref (and natural keys: tax ids, plates, MACs…)
let mut card = Entity::new(Kind::Company, &owner.name).with_ref("vigia", owner.id.to_string());
card.company = Some(Company { tax_ids: vec![TaxId::new("CO", "NIT", &owner.nit)], ..Default::default() });
let up = reg.upsert("vigia", card)?;        // finds an existing card by id / ref / NIT, else creates
store_shared_id(owner.id, up.id);           // keep the UUID next to your record

// pull: what other apps changed since your cursor
loop {
    let batch = reg.pull("vigia", 500)?;
    for s in &batch.items {
        match s.change.op {
            Op::Upsert => apply_card(s.entity.as_ref().unwrap()),        // idempotent, keyed by id
            Op::Merge => repoint(s.change.id, s.change.into.unwrap()),  // the old id became `into`
            Op::Delete => forget_shared_id(s.change.id),                // keep your own record
        }
    }
    reg.ack("vigia", batch.cursor)?;
    if !batch.more { break }
}
```

- `Error::InUseElsewhere(info)`: the data root is in a synced folder and another computer is
  using the registry right now (`info.host`); skip this sync quietly or show a small notice.
- When to sync: on start, after local edits (push just that record), and on a timer (every
  few minutes) or window activation. Run it off the UI thread; each call is one short
  transaction, so other apps never wait long.
- `upsert` unites lists and lets set fields win; it never removes data. Use `replace` only for
  an explicit user edit of the shared card.
- Refs must be unique within your app (one id space for all your kinds, e.g. prefix them:
  `owner-12`, `item-7`).
- Never delete your own records because the shared card was deleted or merged; only update
  the link to it. Never merge on your own: show `duplicates()` to the user or leave it to
  Otherwise Apps' Entities view.
- What to share: owners / counterparties (people, companies), vehicles, properties, devices,
  sites, domains. Put app-only extras under `ext["<id>"]`.

## 4. Expose MCP (proxied by `otherwise-hub`)

1. Add a `mcp` subcommand to the app's CLI that runs a stdio MCP server (JSON-RPC 2.0, one
   message per line: `initialize`, `ping`, `tools/list`, `tools/call`). expense-rs's
   `crates/core/src/mcp.rs` is the reference implementation (≈170 lines, no SDK).
2. Read-only by default. Annotate every read tool `"annotations": {"readOnlyHint": true}`;
   tools without it are treated as writes and hidden unless the user enabled hub writes.
   Offer write tools only with `--allow-writes` (the hub passes it only when the user enabled
   writes in Otherwise Apps); record writes so they can be undone.
3. Ship the CLI where discovery finds it: inside the bundle (`Contents/MacOS/<cli>`), next to
   the GUI in the Windows / Linux folder, or on PATH; the name must be in the app's `cli` list
   in `apps::KNOWN` (and `Capability::Mcp` set, with `mcp_args: ["mcp"]`,
   `mcp_write_arg: Some("--allow-writes")`).
4. Tool names: plain `snake_case` without the app prefix (the hub adds `vigia.`). Return
   `structuredContent` plus a text copy, `isError: true` for tool failures.
5. Check: `otherwise-hub list` shows the app and its tool count; in Claude Code
   `claude mcp add otherwise-hub -- otherwise-hub` (or Otherwise Apps' `… otherwise-apps mcp`).

## 5. Where the app keeps its data (storage)

The user chooses one **data root** for all Otherwise apps (default `~/.otherwise`, movable to
OneDrive & co. from Otherwise Apps → Settings). Each app's default folder is
`otherwise_link::paths::app_dir("<id>")` = `<root>/<id>/`. Machine-local settings, caches and
models stay in the app's platform config folder (they aren't data and shouldn't sync).

```rust
use otherwise_link::{paths, storage, lock::{self, Lock, Scope, Holder}, cloud};

let default_db = paths::app_dir("vigia").join("vigia.db");     // new default for new installs

// moving / "Save As": close your connection first for a move
storage::copy_sqlite(&current, &dest)?;   // VACUUM INTO + integrity_check + same content, never overwrites
storage::move_sqlite(&current, &dest)?;   // the same, then deletes the original (+ sidecars)
storage::move_tree(&old_files_dir, &new_files_dir)?;   // attachments folder: rename or verified copy

// opening a document: lock + cloud checks
match Lock::acquire(&path, "vigia", Scope::Process) {
    Ok(l) => keep(l),                                  // heartbeat() every lock::HEARTBEAT_EVERY, drop on close
    Err(Holder::Elsewhere(i)) => ask_open_read_only_or_take_over(&i.host),   // Lock::take_over after "Use here"
    Err(Holder::Here(_)) => tell_already_open(),       // another window / process here
    Err(_) => unreachable!(),
}
storage::ensure_rollback_journal(&conn)?;              // never WAL
let st = storage::status(&path);                       // st.cloud (provider), st.lock, st.conflicts
```

`heartbeat()` returning `Err` means another computer took the file over: stop writing, tell the
user. `relocate::plan(&from, &to)` reports a folder as blocked while a lock in it is live, so an
app that keeps its database in `<root>/<id>/` must hold its lock while it has it open (that's
what keeps Otherwise Apps from moving it underneath you).

### Storage checklist (per app)

- [ ] **Default location** `paths::app_dir("<id>")` (`<root>/<id>/<file>`) for new documents /
      the default database. Remember whether the user chose a custom path; if not, always
      resolve the default fresh (it follows the data root when the user moves it).
- [ ] **Migrate from the old default** (e.g. `~/Library/Application Support/<App>/<file>`): only
      when the user never chose a custom path and the new default doesn't exist yet. Ask on
      first launch with a clear dialog ("Move to the Otherwise data folder / Keep where it is"),
      then `storage::move_sqlite` (+ `storage::move_tree` for attachment folders), verified;
      on "keep", remember the old path as the custom one. Never move without asking; never
      overwrite.
- [ ] **File menu**: *Save As…* (verified copy, then open the copy), *Move to…* (verified move,
      the original deleted only after verification), *Show in Finder* (Explorer / file
      manager), *Open…*, and a **recent files** list where a missing entry offers *Locate…*
      (file picker) and *Remove from list*. CLI flags like `--db` keep working and are never
      remembered.
- [ ] **Cloud safety** when the file is anywhere (cheap, so always): rollback journal
      (`storage::ensure_rollback_journal`), `Lock::acquire` on open (+ `heartbeat` on a timer,
      release on close; "open on <host>" dialog with *Open read-only* / *Use here anyway*),
      `storage::status` → show the provider name next to the path and warn about
      `conflicts` (conflicted copies) with *Show in Finder*.
- [ ] i18n for every new string (es/en/pt/fr) and tests on temp dirs (`Home::at`, `Root::at`,
      `Lock::acquire_as` with a fake `Me` / clock) for the migration and the moves.

## 6. Checklist per app

- [ ] Depend on otherwise-link by tag; rusqlite 0.37.
- [ ] Scheme in Info.plist, .desktop, Windows registration; links received on all three OSes.
- [ ] `entity/<uuid>` + own paths handled, documented in README.
- [ ] Today feed published (translated titles, stable ids, `expires`).
- [ ] Entity push / pull behind `Registry::open_shared` (silent when disabled).
- [ ] `mcp` subcommand, read-only by default, `readOnlyHint` on reads, `--allow-writes`.
- [ ] i18n keys for any new UI (es/en/pt/fr) + guard tests green; tests on temp dirs only.
- [ ] Storage checklist (§5) done.
