# otherwise-link specification, version 1 (library 0.2)

Normative description of the formats Otherwise apps share. The Rust crate `otherwise-link`
implements all of it; an app in another language can follow this document instead.
"MUST", "SHOULD" and "MAY" are as in RFC 2119.

Compatibility rule for everything below: writers MAY add fields; readers MUST ignore fields
they don't know. Renaming, removing or changing the meaning of a field requires a new schema
number, and readers MUST refuse data with a schema number higher than they understand.

## 1. App ids

Every app has a stable id matching `[a-z0-9][a-z0-9-]{0,63}`, never starting with `-`:
`otherwise-apps` (the launcher, "the hub"), `expense`, `vigia`, `nodo`, `genealogy`, `mosaico`,
`diskmap`, `rama`. The id is the deep-link host, the Today file name, the key in an entity's
`refs`, and the MCP tool prefix. `mcp` is reserved for changes made through `otherwise-hub`.

## 2. Data root

All shared files live in one folder per user, the **data root**, which the user may move
(a local folder, OneDrive, Dropbox, iCloud Drive…). Its location is recorded in a pointer file
at a fixed place, `~/.otherwise/config.json` (Windows `%USERPROFILE%\.otherwise\config.json`):

```json
{ "schema": 1, "data_root": "/Users/me/Library/CloudStorage/OneDrive-Personal/Otherwise" }
```

- `data_root`: absolute path. Missing, not absolute, or the file missing / unreadable: the
  default, `~/.otherwise` (the pointer's own folder). Readers also accept `dataRoot`.
- Readers MUST read the pointer again whenever they resolve the root (another app may have
  moved it); it is written atomically, only by the launcher (or a library call it makes).
- `OTHERWISE_DATA_DIR`, when set and non-empty, replaces the whole lookup.

Contents:

```
hub.json                shared settings (§6)
entities.db             entity registry (§4), only after the user opted in
entities.db.lock.json   which computer uses the registry (§2.2)
today/<app id>.json     Today feeds (§5)
today/.dismissed.json   dismissals (§5.3)
<app id>/               the app's default folder for its own data (e.g. vigia/vigia.db)
```

An app that lets the user put its documents elsewhere still uses `<app id>/` as the default for
new ones. Apps' machine-local settings and caches stay in the platform's config folder.

### 2.1 Moving the root and the pre-0.2 folder

Moving: every item of the old root (all of the above except the pointer and lock files) is
copied to the new folder and **verified** (SQLite: `integrity_check`, same `application_id`,
`user_version` and row content; other files: same bytes); nothing at the destination is
overwritten (a name already there blocks the move), and an item whose lock (§2.2) is held by
another live process or another computer blocks it too. Only when every copy verified is the
pointer written; then, for a move, the originals are removed (`hub.json` first). Pointing at a
folder that already holds the data (a second computer on the same synced folder) writes only
the pointer.

Versions before 0.2 used a platform folder: macOS `~/Library/Application Support/Otherwise`,
Windows `%APPDATA%\Otherwise`, Linux `$XDG_DATA_HOME/otherwise` (`~/.local/share/otherwise`).
On first use, readers move its `entities.db`, `today/*` and `hub.json` (in that order) into the
data root with the same verified copy, then remove the originals; a file already at the root
with other content wins and the old one stays where it was. A `.migrated` JSON marker
(`{"schema":1,"to":…,"at":…,"moved":[…],"kept":[…]}`) in the old folder makes this happen once.

### 2.2 Locks and synced folders

A file that one process (or one computer) should write at a time has a lock next to it,
`<file>.lock.json`:

```json
{ "schema": 1, "app": "vigia", "host": "Ana's MacBook Pro", "pid": 4242,
  "scope": "process", "since": 1763700000, "heartbeat": 1763700060 }
```

- The holder rewrites `heartbeat` (Unix seconds) at least every 60 s while it has the file open
  and removes the lock when it closes it.
- A lock is **stale** when its heartbeat is older than 10 minutes, or it names this computer
  and a process that no longer runs; a stale lock may be taken. A fresh lock of another
  computer means "open on <host>": apps MUST NOT write the file without the user's explicit
  confirmation (then they take the lock over).
- `"scope": "computer"`: shared by every process of that computer (used for `entities.db`,
  which several apps open at once); it is not removed on close and simply goes stale. Every
  `Registry::open_shared` refreshes it (at most once a minute) and fails when another computer
  holds it.

SQLite files that may sit in a synced folder use the rollback journal (`journal_mode=DELETE`)
only, never WAL. Sync clients' "conflicted copies" (`name (… conflicted copy …).ext`,
`name.sync-conflict-….ext`, `name 2.ext`, `name (1).ext`, OneDrive's `name-<computer>.ext`)
SHOULD be reported to the user.

The lock keeps two computers from writing the registry at the same time; it is **not** sync:
the sync client still uploads whole files, so after switching computers its latest version may
not have arrived yet. Real multi-device sync is a separate, future layer (`otherwise-sync`).

## 3. Deep links

```
otherwise://<app>/<segment>/<segment>?<key>=<value>&…
otherwise-<app>://<segment>/<segment>?<key>=<value>&…
```

- Both forms denote the same link. Schemes and the app id are case-insensitive and normalized
  to lower case. Empty segments are ignored, a fragment (`#…`) is dropped.
- Segments, keys and values are percent-encoded UTF-8; writers encode every byte outside
  `A-Z a-z 0-9 - . _ ~`. Readers decode `%XX`, and `+` as a space in the query only. Invalid
  encodings or invalid UTF-8 make the link invalid.
- Each app that takes links registers **its own** scheme `otherwise-<app>`. The launcher
  registers `otherwise` and forwards a link for another app to that app's scheme (never back to
  `otherwise`, to avoid loops); if the app is missing it tells the user and offers the install.
  The launcher's own links (`otherwise://otherwise-apps/…`) have no per-app form.
- Openers SHOULD try `otherwise-<app>://` first and fall back to `otherwise://`.

Paths every app SHOULD understand (others are app-defined and documented by the app):

| Path                    | Meaning                                                      |
|-------------------------|--------------------------------------------------------------|
| (empty)                 | Bring the app to the front.                                  |
| `entity/<uuid>`         | Show the app's record for shared entity `<uuid>` (§4); the app finds its record through the card's `refs[<app>]` (after following merges). |

Launcher paths: `today`, `entities`, `entities/<uuid>`, `hub`, `app/<app id>` (its card).

Registration:

- macOS: `Info.plist` gets `CFBundleURLTypes` → one dict with `CFBundleURLName` (the bundle id)
  and `CFBundleURLSchemes` (`otherwise-<app>`). The URL arrives as an Apple event
  (`application:openURLs:`; GPUI `Application::on_open_urls`), also on a cold start.
- Linux: the `.desktop` file's `[Desktop Entry]` gets `MimeType=x-scheme-handler/otherwise-<app>;`
  and its `Exec` takes `%u`; then `xdg-mime default <file>.desktop x-scheme-handler/otherwise-<app>`.
  The URL arrives as a command-line argument.
- Windows: `HKCU\Software\Classes\otherwise-<app>`: default value `URL:<name>`, empty
  `URL Protocol` value, `shell\open\command` default `"<exe>" "%1"`. The URL arrives as an
  argument. A second instance SHOULD hand the link to the running one.

## 4. Entities

### 4.1 Card

```json
{
  "schema": 1,
  "id": "6f0c2c1e-0000-4000-8000-000000000001",
  "kind": "company",
  "name": "Otherwise Technologies LLC",
  "aliases": ["Otherwise"],
  "refs": { "vigia": "entity-12", "expense": "vendor-9" },
  "tags": ["client"],
  "notes": "…",
  "company": { "taxIds": [ { "country": "US", "scheme": "EIN", "value": "123456789" } ], "country": "US" },
  "ext": { "vigia": { "color": "teal" } },
  "createdAt": 1763700000,
  "updatedAt": 1763700000
}
```

| Field       | Type                  | Notes |
|-------------|-----------------------|-------|
| `schema`    | integer               | 1. Missing means 1. |
| `id`        | UUID string           | Assigned once (v4) by the registry, never changed. Missing or nil = new card. |
| `kind`      | string                | `person`, `company`, `vehicle`, `property`, `device`, `site`, `digital`. |
| `name`      | string                | Non-empty after trimming. |
| `aliases`   | string[]              | Other names; never equal to `name`; unique. |
| `refs`      | {app id: string}      | That app's own id for this entity. Refs are unique per app across the registry. |
| `tags`      | string[]              | Free labels. |
| `notes`     | string                | Free text. |
| `<kind>`    | object                | At most one section, and only the one named by `kind`. |
| `ext`       | {app id: any}         | App-specific extras; other apps keep them untouched. |
| `createdAt`, `updatedAt` | Unix seconds | Set by the registry. |

Sections (all fields optional; camelCase):

- `person`: `givenName`, `familyName`, `birthDate`, `deathDate` (`YYYY`, `YYYY-MM` or
  `YYYY-MM-DD`), `emails[]`, `phones[]`, `taxIds[]` (national ids too: CC, SSN, CPF…), `country`.
- `company`: `legalName`, `taxIds[]`, `country`, `website`, `emails[]`, `phones[]`, `address`.
- `vehicle`: `plate`, `country`, `vin`, `make`, `model`, `year` (integer), `color`.
- `property`: `address`, `country`, `cadastralId`, `lat`, `lon`.
- `device`: `macs[]`, `hostnames[]`, `ips[]`, `tailscaleId`, `serial`, `vendor`, `model`, `os`.
- `site`: `lat`, `lon`, `radiusM`, `address`, `country`.
- `digital`: `domain`, `service`, `account`, `url`.

Tax id: `{ "country": ISO 3166-1 alpha-2, "scheme": "NIT" | "CC" | "EIN" | "SSN" | "CNPJ" |
"CPF" | "SIREN" | "SIRET" | "RFC" | "VAT" | …, "value": … }`. Check digits are part of `value`.

Normalization (applied by the registry on every write): names and aliases trimmed; tax-id
country upper case, scheme and value reduced to upper-case letters and digits
(`900.123.456-7` → `9001234567`); emails lower case; plates and VINs upper-case alphanumerics;
vehicle country upper case; MACs `aa:bb:cc:dd:ee:ff` (any of `:`, `-`, `.` or no separator
accepted on input; anything else is an error); hostnames lower case without a trailing dot;
domains lower case without scheme, path or `www.`; service lower case; `lat` in [-90, 90],
`lon` in [-180, 180].

### 4.2 Natural keys

Strings that identify the same real-world thing across apps:

| Key                                  | From |
|--------------------------------------|------|
| `tax:<country>:<scheme>:<value>`     | person / company tax ids |
| `email:<email>`                      | person emails |
| `plate:<country>:<plate>`, `vin:<vin>` | vehicle |
| `cadastral:<country>:<id>`           | property |
| `mac:<mac>`, `tailscale:<id>`        | device |
| `account:<service>:<account>`, else `domain:<domain>` | digital |

A key belongs to at most one live card (first writer wins; the other card simply doesn't get
it, and the pair shows up as a merge candidate).

### 4.3 Registry file `entities.db`

SQLite 3, `PRAGMA application_id = 0x4F57454E`, `user_version` = number of migrations applied
(1 in this version). Opt-in: nothing creates or writes it unless `hub.json` has
`"entities": true`.

Concurrency: every process opens its own connection with the rollback journal
(`journal_mode=DELETE`; WAL MUST NOT be enabled), a busy timeout of at least 10 s, and does
every write as one `BEGIN IMMEDIATE` transaction that includes finding the existing card. No
transaction is held across user interaction or network I/O.

Tables (version 1):

```sql
entities(id TEXT PRIMARY KEY, kind TEXT, name TEXT, search TEXT, card TEXT,  -- card = §4.1 JSON
         created_at INTEGER, updated_at INTEGER, deleted_at INTEGER, merged_into TEXT)
keys(key TEXT PRIMARY KEY, entity_id TEXT)          -- §4.2
refs(app TEXT, ref TEXT, entity_id TEXT, PRIMARY KEY (app, ref))
changes(seq INTEGER PRIMARY KEY AUTOINCREMENT, entity_id TEXT, op TEXT, into_id TEXT, app TEXT, at INTEGER)
cursors(app TEXT PRIMARY KEY, seq INTEGER, at INTEGER)
```

`search` is the folded (lower case, accents removed, punctuation → spaces) text of name,
aliases, keys, tags, legal name, hostnames, IPs and domain, joined by ` | `.

Operations (each appends to `changes`, `op` ∈ `upsert`, `delete`, `merge`):

- **upsert** (app, card): find the existing live card, in order: by `id` (following
  `merged_into`), by `refs[app]`, by any other `refs` entry, by any natural key; matches of a
  different kind are skipped (an explicit `id` of another kind is an error). Found: the card is
  updated (incoming non-null fields win, arrays are united keeping order, objects merge
  recursively, a rename doesn't make the old name an alias). Not found: inserted with a fresh v4
  id (a caller-supplied id is kept unless it was ever used). Logs `upsert`.
- **replace** (app, card): sets the card exactly (user edit); keys and refs it no longer has
  are released. Logs `upsert`.
- **link / unlink** (app, id, ref): sets / removes `refs[app]`. Logs `upsert`.
- **merge** (app, keep, drop): `keep` absorbs `drop` (keep's fields win, drop fills the gaps,
  arrays / refs / ext unite, drop's name and aliases become aliases), drop's keys and refs move
  to keep, drop gets `merged_into = keep` and `deleted_at`. Logs `merge` (entity = drop,
  into = keep) then `upsert` (keep).
- **delete** (app, id): tombstone (`deleted_at`), its keys and refs are released. Logs `delete`.

### 4.4 Sync

`changes.seq` increases by one per change and becomes visible in order (writers are
serialized). An app keeps a cursor (`cursors.seq`, starting at 0):

1. **pull**: read changes with `seq > cursor` (a bounded batch), group by entity, keep the
   latest change per entity, skip entities whose changes in the batch were all made by this app,
   and return each with the card's current state (none for `delete` / `merge`; a `merge` names
   the survivor in `into`).
2. Apply them locally (idempotently: key by entity id).
3. **ack**: set the cursor to the batch's last `seq` (never moves backwards).

A crash between 2 and 3 replays the batch. Apps push their own changes with upsert / link at
any time; they SHOULD store the shared `id` next to their record once known.

## 5. Today feed

### 5.1 File

`today/<app id>.json`, written only by that app, always whole and atomically (write a temp file
in the same folder whose name starts with `.` and ends in `.tmp`, flush, rename over the target).

```json
{ "schema": 1, "app": "vigia", "updatedAt": "2026-11-21T08:00:00Z",
  "events": [
    { "id": "item-42-due", "kind": "due", "title": "SOAT ABC123 vence", "body": "…",
      "when": "2026-11-30", "severity": "high", "link": "otherwise://vigia/item/42",
      "expires": "2026-12-07", "entity": "6f0c…" } ] }
```

| Field      | Notes |
|------------|-------|
| `id`       | Required, unique within the file, stable across rewrites (dismissals key on it). |
| `kind`     | `due` (has a deadline), `alert` (wrong now), `review` (waits for the user), `info`. |
| `title`    | Required; readers cut it at 200 characters. |
| `body`     | Optional detail. |
| `when`     | `YYYY-MM-DD` (a local day) or RFC 3339 (an instant). Missing: undated. |
| `severity` | `low`, `normal` (default), `high`, `critical`. |
| `link`     | An `otherwise://` link that opens the thing. |
| `expires`  | Same formats as `when`; a day lasts until the end of that local day. Expired events are not shown. |
| `entity`   | Shared entity id it is about. |

Limits: at most 200 events per file (readers ignore the rest), at most 1 MiB per file.

### 5.2 Reading

Readers list `today/`, take files named `<valid app id>.json`, skip files with a newer
`schema` or that don't parse (reporting them), set every event's `app` from the **file name**,
drop expired and dismissed events, and sort: by time (a day sorts at its local midnight;
undated last), then severity (highest first), then app id, then event id. Grouping by local
day; days before today are shown as overdue.

### 5.3 Dismissals

`today/.dismissed.json`: `{"<app id>/<event id>": <Unix seconds>, …}`, written by the launcher
(atomically). Entries older than 60 days are pruned when a new one is added.

## 6. `hub.json`

```json
{ "schema": 1, "entities": false, "mcpWrites": false, "mcpDisabled": [] }
```

- `entities`: the user turned the shared registry on. Default false.
- `mcpWrites`: `otherwise-hub` offers write tools. Default false.
- `mcpDisabled`: app ids whose MCP servers the hub doesn't start.

Written by the launcher only (atomically); missing or unreadable means all defaults.

## 7. Discovery

| OS      | Where an app is looked for |
|---------|----------------------------|
| macOS   | `/Applications/<appName>.app`, `~/Applications/<appName>.app`; `CFBundleIdentifier` MUST equal the app's bundle id; version = `CFBundleShortVersionString`; executables in `Contents/MacOS`. |
| Windows | `%LOCALAPPDATA%\Programs\Otherwise\<installName>\<binary>.exe`, then `%ProgramFiles%\Otherwise\…` |
| Linux   | `~/.local/opt/otherwise/<installName>/<binary>`, then `/opt/otherwise/…` |
| all     | CLI binaries next to the GUI, else on `PATH` (plus `~/.local/bin`, `~/.cargo/bin`, `/opt/homebrew/bin`, `/usr/local/bin`). |

On Windows / Linux the version comes from `.otherwise-install.json` (`{"id": …, "version": …}`)
written by the launcher next to the binary. The built-in table (`apps::KNOWN`) lists id, name,
bundle id, bundle / folder names, binaries, MCP arguments and capabilities (`deepLinks`,
`today`, `entities`, `mcp`).

## 8. MCP

An app exposing MCP runs a stdio server (JSON-RPC 2.0, one message per line) as
`<cli> mcp`, read-only by default, with write tools only under a flag (`--allow-writes`).
Read tools MUST be annotated `"readOnlyHint": true`; tools without it are treated as writes.

`otherwise-hub` starts each installed app's server (unless disabled in `hub.json`), passing the
write flag only when `mcpWrites` is on, and offers:

- every child tool as `<app id>.<tool>` (calls also accept `<app id>__<tool>`), description
  prefixed with `[<app name>]`; tools not annotated read-only only when writes are on;
- `entities_search`, `entities_get`, `today`, `open_link`; with writes on, `entities_upsert` and
  `entities_link` (registry changes are recorded with app id `mcp`).

Writes are re-checked against `hub.json` on every call. A child that fails to start within 15 s
is skipped; one that died is restarted once per call; a call that times out (120 s) kills it.
