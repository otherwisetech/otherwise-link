# otherwise-link specification, version 1 (library 0.5)

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
<app id>/events.json    the app's calendar events (§9)
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
- `background` (library 0.4): `{"quiet": bool, "jobsOff": [app id…], "notificationsOff": [app
  id…]}` for Otherwise Helper (§11). Defaults: not quiet, everything on.

Readers that rewrite the file MUST keep fields they don't know (library 0.4 does).

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
`today`, `entities`, `mcp`, `calendar`).

### 7.1 Apps that describe themselves (library 0.5.1)

An app not in the built-in table is found by a file in its install, so a new app needs no
otherwise-link release: `Contents/Resources/otherwise-app.json` in a macOS bundle (any
`*.app` in the folders above), `<root>/<folder>/otherwise-app.json` on Windows / Linux.
At most 16 KiB:

```json
{"schema": 1, "id": "surco-riego", "name": "Surco Riego",
 "bundleId": "com.otherwisetech.surco.riego", "binary": "surco-riego",
 "cli": ["surco-riego"], "mcpArgs": ["mcp"], "mcpWriteArg": "--allow-writes",
 "openSource": false, "capabilities": ["deepLinks", "mcp", "calendar"]}
```

- `schema` MUST be 1 (others are ignored); `id` a valid app id (§1); `bundleId` MUST start
  with `com.otherwisetech.` or `io.github.otherwisetech.`; `binary` and `cli` are plain file
  names. On macOS the bundle's `CFBundleIdentifier` MUST equal `bundleId`.
- The bundle name (without `.app`) or the folder name becomes `appName` / `installName`.
- Ids of the built-in table can't be redefined; the first install of an id wins.
- A built-in app's own install MAY ship the file too (library 0.5.2): when its `id` and
  `bundleId` equal the table entry's, and it sits in that entry's bundle (`<appName>.app`,
  `CFBundleIdentifier` equal) or folder (`<installName>`, next to its binary), it fills only
  what the entry leaves empty: `cli`, `mcpArgs`, `mcpWriteArg`, `capabilities`. Values the
  table sets are never replaced; name, bundle id, binary and `openSource` always come from the
  table (`apps::known_spec_at`).
- Unknown capabilities and fields are ignored. Discovery (`apps::discover`) and background
  jobs (§11, `background::scan`) both use the table plus these (`apps::all_specs`).

## 8. MCP

An app exposing MCP runs a stdio server (JSON-RPC 2.0, one message per line) as
`<cli> mcp`, read-only by default, with write tools only under a flag (`--allow-writes`).
Read tools MUST be annotated `"readOnlyHint": true`; tools without it are treated as writes.

`otherwise-hub` starts each installed app's server (unless disabled in `hub.json`), passing the
write flag only when `mcpWrites` is on, and offers:

- every child tool as `<app id>.<tool>` (calls also accept `<app id>__<tool>`), description
  prefixed with `[<app name>]`; tools not annotated read-only only when writes are on;
- `entities_search`, `entities_get`, `today`, `calendar_events` (§9.4), `open_link`; with writes on, `entities_upsert` and
  `entities_link` (registry changes are recorded with app id `mcp`).

An app MAY refuse the write flag (exit non-zero at start, with the reason on stderr), for
example when its own settings don't allow agent changes; the hub (0.5.3+) then starts it again
without the flag and offers only its read tools for the rest of that hub session (never the
other way round). An app that fails both ways is skipped with its error.
Writes are re-checked against `hub.json` on every call. A child that fails to start within 15 s
is skipped; one that died is restarted once per call; a call that times out (120 s) kills it.

## 9. Events feed (calendar)

Added in library 0.3. The Today feed (§5) says what needs attention now; the events feed says
what is on the calendar (due dates, renewals, planned payments, birthdays, audits…), for
Otherwise Apps' Calendar view, its combined ICS file and the hub's `calendar_events` tool.

### 9.1 File

`<data root>/<app id>/events.json`, written only by that app, whole and atomically (as §5.1).
Writers SHOULD skip the write when the list didn't change.

```json
{ "schema": 1, "app": "vigia", "updatedAt": "2026-11-21T08:00:00Z",
  "window": { "from": "2026-11-01", "to": "2027-11-30" },
  "events": [
    { "id": "item-42", "title": "SOAT ABC123 vence", "start": "2026-11-30", "kind": "due",
      "link": "otherwise://vigia/item/42", "entities": ["6f0c…"] },
    { "id": "rent", "title": "Arriendo", "start": "2026-01-05", "rrule": "FREQ=MONTHLY",
      "kind": "payment", "status": "confirmed", "color": "orange", "busyOnly": true } ] }
```

| Field       | Notes |
|-------------|-------|
| `id`        | Required, unique within the file, stable across rewrites. |
| `title`     | Required, in the user's language; readers cut it at 200 characters. |
| `body`, `location` | Optional text. |
| `start`     | Required. `YYYY-MM-DD` (all day) or RFC 3339 with an offset (a moment). An event whose `start` doesn't parse is ignored. |
| `end`       | Optional, same type as `start`. All day: the **last day, inclusive** (default: `start`). Timed: the end instant, exclusive (default: `start`). |
| `timezone`  | Optional IANA name the event was made in; informational (the offsets rule). |
| `rrule`     | Optional recurrence, the subset of §9.2. Readers that can't parse it show the first occurrence only. |
| `exdates`   | Days (`YYYY-MM-DD`; an RFC 3339 value counts by its day in the start's offset) the rule skips. |
| `series`    | Optional: groups expanded instances, for apps that write occurrences instead of a rule. |
| `kind`      | `due`, `renewal`, `reminder`, `payment`, `payroll`, `tax`, `close`, `birthday`, `anniversary`, `meeting`, `booking`, `task`, `audit`, `maintenance`, `warranty`, `holiday`, `other` (default; unknown kinds read as `other`). |
| `category`  | Optional app label, already translated. |
| `status`    | `confirmed` (default; unknown values read as it), `tentative`, `done`, `cancelled` (readers leave it out). |
| `color`     | Optional hint: `#rrggbb` or `red`, `orange`, `yellow`, `green`, `teal`, `blue`, `purple`, `pink`, `gray`. Readers may color by app instead. |
| `entities`  | Shared entity ids (§4) it is about. |
| `link`      | `otherwise://<app>/…` that opens the thing (an `https` URL for web sources). |
| `busyOnly`  | Private: readers show the title only in the user's own calendar UI. Exports (ICS), assistants and MCP get a generic title ("Busy") without `body`, `location`, `category`. Writers SHOULD keep amounts and other sensitive details out of `title` when the user asked for privacy. |
| `window`    | Optional (feed level): the range expanded instances cover. |

Limits: at most 5000 events per file, 4 MiB per file; one event yields at most 1000
occurrences per read. Writers SHOULD publish rules rather than long instance lists, and keep
expanded instances to about a year ahead.

### 9.2 Recurrence

`FREQ=DAILY|WEEKLY|MONTHLY|YEARLY` with `INTERVAL` (≥ 1), `COUNT`, `UNTIL` (`YYYYMMDD`, a
day: occurrences may start on it), `BYDAY` (`MO,WE`; in MONTHLY / YEARLY `2MO`, `-1FR` = the
nth weekday of the month), `BYMONTH`, `BYMONTHDAY` (`-1` = last day). Other parts are ignored;
another FREQ makes the rule unusable. Week starts on Monday. Days a rule names that don't exist
(Feb 30, Feb 29 in a common year) are skipped, as in RFC 5545. A timed rule repeats the
start's wall-clock time in the start's **offset** (no time-zone database): writers whose
events cross a DST change SHOULD write instances instead.

### 9.3 Reading

Readers list the data root's folders named by a valid app id, read `<folder>/events.json`,
skip newer schemas and unparsable files (reporting them), set every event's `app` from the
**folder name**, drop `cancelled` events, expand rules for the requested range of local days
and sort by first local day, all-day before timed, then start time, app id, event id. An
occurrence's key is `<app>/<id>/<start>`. Holidays merged by readers use the pseudo app id
`holidays`.

### 9.4 ICS and the hub

`ics::write` exports events as iCalendar: UID `<id>.<app>@otherwise.local`, all-day events
as `VALUE=DATE` with an exclusive `DTEND`, timed ones in UTC, `RRULE` / `EXDATE` kept, `URL`
= the link, busy-only events as `SUMMARY:<busy label>` + `CLASS:PRIVATE` without details.
`ics::parse` reads calendars into events (overrides by `RECURRENCE-ID` replace their
occurrence; `TZID` / floating times are read in the reader's zone).

`calendar_events` (read-only): `{from?, to?, apps?, holidays?}` → occurrences between two
local days (default today + 6 days, at most 400 days), busy-only events redacted.

## 10. Holidays

`holidays` computes public holidays from their rules (no data, no network): `co` (Ley
Emiliani moves, Easter-based days), `us` (federal, observed days), `us-wy` (Wyoming state
offices), `es`, `mx`, `br`, `ca`. Each has a stable id (`co_new_year`, `us_thanksgiving`…)
for apps to translate and its proper name in the country's language. Apps that show holidays
or shift dates to business days SHOULD use it so they agree.

## 11. Background jobs (Otherwise Helper)

Added in library 0.4. Otherwise Helper is the launcher's small menu-bar process; it runs work
apps declare (check mail, ping devices, refresh calendars) while none of their windows is open,
and shows their notifications. It never links an app's code: it only runs the app's own
executables.

### 11.1 Declaration

Inside the install, never in the data root, so nothing has to be registered and removing the
app removes its jobs:

- macOS: `<App>.app/Contents/Resources/otherwise-background.json`
- Windows / Linux: `otherwise-background.json` in the install folder (§7)

```json
{ "schema": 1, "app": "expense", "jobs": [
  { "id": "mail", "label": { "key": "background.mail", "en": "Check mail", "es": "Revisar correo" },
    "every": "15m", "exe": "expense", "args": ["background", "run", "mail"],
    "timeoutSecs": 300, "memoryMb": 400, "whileOpen": true, "heavy": true, "network": true } ] }
```

- `app` MUST be the install's app id; `schema` newer than 1: the file is ignored. At most 64 KiB
  and 16 jobs.
- `id`: `[a-z0-9-]`, at most 40 characters, unique. `label`: `en` required, other languages by
  code (`es`, `pt`, `fr`), `key` optional.
- `every` (`30s`, `15m`, `1h`, `1d`, at least one minute) or `daily` (`HH:MM`, local time). A
  job without either is ignored.
- `exe`: a file name in `Contents/MacOS` (Windows / Linux: the install folder, `.exe` implied);
  default the app's GUI binary. Paths, `..` and symlinks resolving outside are refused.
- `timeoutSecs` (default 300, at most 3600), `memoryMb` (default 512, at most 4096): the helper
  kills a run that exceeds them.
- `whileOpen` (default true): run even while the app is open. `heavy`: never two at once,
  skipped under memory pressure. `network`: skipped while offline.

### 11.2 Running

The helper scans the installed apps (§7, bundle or install folder only — a CLI on `PATH`
doesn't count; on macOS the bundle's `CFBundleIdentifier` must be the app's) when it starts and
periodically. **Before every run** it repeats the detection and runs nothing unless the same
install still exists, still declares the job and the executable is an executable file inside
it. An app that is gone is shown as not installed (or hidden) and not scheduled again until a
later scan finds it; nothing is downloaded or installed, and the app's data folder is left as
it is.

A run gets `OTHERWISE_BACKGROUND_JOB=<id>` and `OTHERWISE_LANG=<es|en|pt|fr>` in its
environment, no stdin, and the user's data root as usual. **Single writer:** a job that writes
MUST send its writes through the app when the app has the data open (its local socket), and
otherwise take the data's lock (§2.2) itself. A job with nothing configured exits 0 and reports
`idle`; it never notifies about that.

Exit 0 is success; anything else (or a kill) is a failure, retried with backoff and shown in
the helper's menu, never as a notification. The last stdout line may be a report:

```json
{"otherwiseJob": 1, "status": "2 new receipts", "idle": false}
```

`status` (at most 120 characters, in the user's language) is shown in the helper's menu.

### 11.3 One computer per data root (library 0.5)

A data root in a synced folder may be used from several computers, each with its own helper.
Per app, only the computer holding the **lease** runs that app's jobs and drains its outbox:
`<data root>/<app id>/background.lock.json`, a §2.2 lock with scope `computer` and app
`otherwise-helper`, refreshed at most once a minute while the helper runs; another computer's
lease older than 10 minutes is free to take. The others wait and show "running on <host>".
The lease is not an open file: moving the root (§2.1) ignores it.

What a helper remembers about its own runs (last runs, backoff, pause, shown notifications) is
the computer's, never the data root's: `paths::device_dir()` —
`~/Library/Application Support/com.otherwisetech.device/` (macOS),
`%LOCALAPPDATA%\Otherwise\device\` (Windows), `$XDG_STATE_HOME/otherwise/` (Linux,
default `~/.local/state/otherwise/`); `OTHERWISE_DEVICE_DIR` overrides it. It is never the
pre-0.2 folder (§2.1) and never moves with the root; a `helper/` folder an Otherwise Apps 0.6.0
helper left at the top of the root is device state too and is left out of a move.

### 11.4 Notifications

`<data root>/<app id>/outbox/<id>.json`, written by the app (atomically, as §5.1), one file per
notification:

```json
{ "schema": 1, "id": "bill-42-due", "title": "Internet due tomorrow", "body": "…",
  "link": "otherwise://expense/payable/42", "severity": "high",
  "createdAt": "2026-11-23T08:00:00Z" }
```

The lease holder's helper (§11.3) drains the folder (reads, then removes each file) only for installed apps, drops
notifications older than 24 hours, shows each `<app>/<id>` once (for 30 days), and opens
`link` when one is clicked. Nothing is shown when `hub.json` `background.quiet` is on or the
app is in `notificationsOff`; jobs of apps in `jobsOff` don't run.
