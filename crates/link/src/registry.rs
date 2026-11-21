//! The shared entity registry: one SQLite file (`entities.db` in the shared folder) that every
//! app may read and write, so a company typed into expense shows up in Vigía and a device found
//! by Nodo can carry Vigía's warranty dates.
//!
//! **Opt-in.** [`Registry::open_shared`] refuses ([`Error::Disabled`]) until the user turns the
//! registry on in Otherwise Apps (`hub.json` → `entities: true`); nothing creates the file
//! before that.
//!
//! **Several processes at once.** Every app opens its own connection. The database uses the
//! rollback journal (`journal_mode=DELETE`, never WAL: WAL's shared-memory index misbehaves on
//! network and synced folders, and the other apps' databases follow the same rule), a 10 s
//! `busy_timeout`, and every write is one `BEGIN IMMEDIATE` transaction, so writers queue up on
//! SQLite's file lock instead of failing with `SQLITE_BUSY`, and matching ("is this the same
//! company?") happens inside the same transaction as the insert. Keep transactions short: no
//! network or UI work while one is open (the API never holds one across calls).
//!
//! **Change log and sync.** Every write appends to `changes` (`seq` grows by one per change and
//! commits in order, since writers are serialized). Each app keeps a cursor: [`Registry::pull`]
//! returns what *other* apps changed after it (collapsed to the latest state per entity), and
//! [`Registry::ack`] moves the cursor once the app has applied them. A crash between the two
//! just replays the batch, so applying must be idempotent (it is, if the app keys by entity id).
//!
//! **Identity.** Ids are UUIDs assigned once. [`Registry::upsert`] finds the existing card by
//! id, then the caller's own ref, other apps' refs, then natural keys (tax id, plate, MAC…,
//! see [`Entity::natural_keys`]); only then does it create one. [`Registry::merge`] folds a
//! duplicate into another card; the old id keeps resolving to the survivor.
use crate::entity::{CardError, Entity, Kind};
use crate::paths::Root;
use crate::settings::HubSettings;
use crate::text::fold;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

/// `PRAGMA application_id`: "OWEN" (Otherwise ENtities).
pub const APPLICATION_ID: i32 = 0x4F57_454E;
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// Schema steps; `user_version` = how many ran. Append only, never edit a shipped one.
const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE entities (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  search TEXT NOT NULL,
  card TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  deleted_at INTEGER,
  merged_into TEXT
);
CREATE INDEX entities_kind_name ON entities(kind, name);
CREATE TABLE keys (key TEXT PRIMARY KEY, entity_id TEXT NOT NULL);
CREATE INDEX keys_entity ON keys(entity_id);
CREATE TABLE refs (app TEXT NOT NULL, ref TEXT NOT NULL, entity_id TEXT NOT NULL, PRIMARY KEY (app, ref));
CREATE INDEX refs_entity ON refs(entity_id);
CREATE TABLE changes (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  entity_id TEXT NOT NULL,
  op TEXT NOT NULL,
  into_id TEXT,
  app TEXT NOT NULL,
  at INTEGER NOT NULL
);
CREATE TABLE cursors (app TEXT PRIMARY KEY, seq INTEGER NOT NULL, at INTEGER NOT NULL);
"#];

#[derive(Debug)]
pub enum Error {
    /// The user hasn't turned the shared registry on.
    Disabled,
    Sql(rusqlite::Error),
    Card(CardError),
    NotFound(Uuid),
    /// Written by a newer version of this library.
    NewerDatabase(i64),
    NotOurs,
    KindMismatch { id: Uuid, existing: Kind, given: Kind },
    BadApp(String),
    /// Another computer is using this registry (it sits in a synced folder); see [`crate::lock`].
    InUseElsewhere(crate::lock::LockInfo),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Disabled => write!(f, "the shared entity registry is off (turn it on in Otherwise Apps → Entities)"),
            Error::Sql(e) => write!(f, "entity registry: {e}"),
            Error::Card(e) => write!(f, "{e}"),
            Error::NotFound(id) => write!(f, "no entity {id}"),
            Error::NewerDatabase(v) => write!(f, "entities.db is from a newer version (schema {v}); update the app"),
            Error::NotOurs => write!(f, "this file is not an Otherwise entity registry"),
            Error::KindMismatch { id, existing, given } => write!(f, "entity {id} is a {}, not a {}", existing.key(), given.key()),
            Error::BadApp(a) => write!(f, "invalid app id {a:?}"),
            Error::InUseElsewhere(i) => write!(f, "the shared entity registry is in use on {} (it is in a synced folder)", i.host),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::Sql(e)
    }
}

impl From<CardError> for Error {
    fn from(e: CardError) -> Error {
        Error::Card(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Upsert,
    Delete,
    /// `id` was folded into `into`.
    Merge,
}

impl Op {
    fn key(self) -> &'static str {
        match self {
            Op::Upsert => "upsert",
            Op::Delete => "delete",
            Op::Merge => "merge",
        }
    }

    fn parse(s: &str) -> Op {
        match s {
            "delete" => Op::Delete,
            "merge" => Op::Merge,
            _ => Op::Upsert,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub seq: i64,
    pub op: Op,
    pub id: Uuid,
    pub into: Option<Uuid>,
    /// The app that made it.
    pub app: String,
    pub at: i64,
}

/// One entity in a [`Pull`]: the latest change to it and its state now (None once deleted or
/// merged away; then `into` names the survivor for a merge).
#[derive(Debug, Clone, PartialEq)]
pub struct Synced {
    pub change: Change,
    pub entity: Option<Entity>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pull {
    pub items: Vec<Synced>,
    /// Pass to [`Registry::ack`] after applying `items`.
    pub cursor: i64,
    /// There are more changes after `cursor`.
    pub more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matched {
    Created,
    Id,
    OwnRef,
    OtherRef,
    Key,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Upserted {
    pub id: Uuid,
    pub matched: Matched,
    pub entity: Entity,
}

pub struct Registry {
    conn: Connection,
    path: PathBuf,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn check_app(app: &str) -> Result<()> {
    if crate::link::valid_app_id(app) { Ok(()) } else { Err(Error::BadApp(app.to_string())) }
}

fn uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).unwrap_or_default()
}

impl Registry {
    /// The user's shared registry, if they turned it on.
    pub fn open_shared(root: &Root) -> Result<Registry> {
        if !HubSettings::load(root).entities {
            return Err(Error::Disabled);
        }
        // one computer at a time when the folder is shared through a sync client
        let app = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default();
        let lock = crate::lock::Lock::acquire(&root.entities_db(), &app, crate::lock::Scope::Computer).map_err(|h| match h {
            crate::lock::Holder::Elsewhere(i) | crate::lock::Holder::Here(i) => Error::InUseElsewhere(i),
            _ => Error::Disabled,
        })?;
        lock.forget();
        Registry::open(&root.entities_db())
    }

    /// Opens (creating and migrating) the registry at `path`, regardless of the opt-in.
    pub fn open(path: &Path) -> Result<Registry> {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        retry(|| {
            // setting the mode takes a lock even when nothing changes: only when it differs
            crate::storage::ensure_rollback_journal(&conn)?;
            Ok(())
        })?;
        retry(|| migrate(&mut conn))?;
        Ok(Registry { conn, path: path.to_path_buf() })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Follows merges to the live card `id` became; None if unknown or deleted.
    pub fn resolve(&self, id: Uuid) -> Result<Option<Uuid>> {
        resolve(&self.conn, id)
    }

    pub fn get(&self, id: Uuid) -> Result<Option<Entity>> {
        match resolve(&self.conn, id)? {
            Some(live) => load(&self.conn, live),
            None => Ok(None),
        }
    }

    /// The card `app` calls `r`.
    pub fn by_ref(&self, app: &str, r: &str) -> Result<Option<Entity>> {
        let id: Option<String> = self.conn.query_row("SELECT entity_id FROM refs WHERE app = ?1 AND ref = ?2", params![app, r], |row| row.get(0)).optional()?;
        match id {
            Some(id) => self.get(uuid(&id)),
            None => Ok(None),
        }
    }

    /// The card holding natural key `key` (`tax:CO:NIT:…`).
    pub fn by_key(&self, key: &str) -> Result<Option<Entity>> {
        let id: Option<String> = self.conn.query_row("SELECT entity_id FROM keys WHERE key = ?1", [key], |row| row.get(0)).optional()?;
        match id {
            Some(id) => self.get(uuid(&id)),
            None => Ok(None),
        }
    }

    /// Creates or updates `card` on behalf of `app` (see the module docs for how the existing
    /// card is found). An update unites lists and lets `card`'s set fields win; use
    /// [`Registry::replace`] to remove things.
    pub fn upsert(&mut self, app: &str, card: Entity) -> Result<Upserted> {
        retry(|| self.upsert_once(app, card.clone()))
    }

    fn upsert_once(&mut self, app: &str, card: Entity) -> Result<Upserted> {
        check_app(app)?;
        let mut card = card;
        card.normalize()?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let found = find_existing(&tx, app, &card)?;
        let t = now();
        let (entity, matched) = match found {
            Some((id, matched)) => {
                let mut existing = load(&tx, id)?.ok_or(Error::NotFound(id))?;
                if existing.kind != card.kind {
                    return Err(Error::KindMismatch { id, existing: existing.kind, given: card.kind });
                }
                existing.apply(&card);
                existing.updated_at = t;
                (existing, matched)
            }
            None => {
                if !card.id.is_nil() && exists_any(&tx, card.id)? {
                    // a deleted id is not reused for something else
                    card.id = Uuid::nil();
                }
                if card.id.is_nil() {
                    card.id = Uuid::new_v4();
                }
                card.created_at = t;
                card.updated_at = t;
                (card, Matched::Created)
            }
        };
        store(&tx, &entity, matched == Matched::Created)?;
        log(&tx, Op::Upsert, entity.id, None, app, t)?;
        tx.commit()?;
        Ok(Upserted { id: entity.id, matched, entity })
    }

    /// Sets the card exactly as given (the user edited it). `card.id` must exist.
    pub fn replace(&mut self, app: &str, card: Entity) -> Result<Entity> {
        retry(|| self.replace_once(app, card.clone()))
    }

    fn replace_once(&mut self, app: &str, card: Entity) -> Result<Entity> {
        check_app(app)?;
        let mut card = card;
        card.normalize()?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = resolve(&tx, card.id)?.ok_or(Error::NotFound(card.id))?;
        let old = load(&tx, id)?.ok_or(Error::NotFound(id))?;
        card.id = id;
        card.created_at = old.created_at;
        card.updated_at = now();
        store(&tx, &card, false)?;
        // keys the card no longer has are freed; refs removed from the card stop resolving
        let keys = card.natural_keys();
        let held: Vec<String> = query_strings(&tx, "SELECT key FROM keys WHERE entity_id = ?1", &id.to_string())?;
        for k in held.iter().filter(|k| !keys.contains(k)) {
            tx.execute("DELETE FROM keys WHERE key = ?1", [k])?;
        }
        for (a, r) in &old.refs {
            if card.refs.get(a) != Some(r) {
                tx.execute("DELETE FROM refs WHERE app = ?1 AND ref = ?2 AND entity_id = ?3", params![a, r, id.to_string()])?;
            }
        }
        log(&tx, Op::Upsert, id, None, app, card.updated_at)?;
        tx.commit()?;
        Ok(card)
    }

    /// Records that `app` calls entity `id` by `r` (sets `refs[app]`).
    pub fn link(&mut self, app: &str, id: Uuid, r: &str) -> Result<Entity> {
        retry(|| self.link_once(app, id, r))
    }

    fn link_once(&mut self, app: &str, id: Uuid, r: &str) -> Result<Entity> {
        check_app(app)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = resolve(&tx, id)?.ok_or(Error::NotFound(id))?;
        let mut e = load(&tx, id)?.ok_or(Error::NotFound(id))?;
        e.refs.insert(app.to_string(), r.to_string());
        e.updated_at = now();
        store(&tx, &e, false)?;
        log(&tx, Op::Upsert, id, None, app, e.updated_at)?;
        tx.commit()?;
        Ok(e)
    }

    /// Drops `app`'s ref from entity `id`.
    pub fn unlink(&mut self, app: &str, id: Uuid) -> Result<Entity> {
        retry(|| self.unlink_once(app, id))
    }

    fn unlink_once(&mut self, app: &str, id: Uuid) -> Result<Entity> {
        check_app(app)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = resolve(&tx, id)?.ok_or(Error::NotFound(id))?;
        let mut e = load(&tx, id)?.ok_or(Error::NotFound(id))?;
        e.refs.remove(app);
        e.updated_at = now();
        store(&tx, &e, false)?;
        tx.execute("DELETE FROM refs WHERE app = ?1 AND entity_id = ?2", params![app, id.to_string()])?;
        log(&tx, Op::Upsert, id, None, app, e.updated_at)?;
        tx.commit()?;
        Ok(e)
    }

    /// Folds `drop` into `keep`: `keep`'s fields win, `drop` fills gaps, lists / refs / keys unite,
    /// `drop`'s name becomes an alias. `drop` then resolves to `keep`.
    pub fn merge(&mut self, app: &str, keep: Uuid, drop: Uuid) -> Result<Entity> {
        retry(|| self.merge_once(app, keep, drop))
    }

    fn merge_once(&mut self, app: &str, keep: Uuid, drop: Uuid) -> Result<Entity> {
        check_app(app)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let keep = resolve(&tx, keep)?.ok_or(Error::NotFound(keep))?;
        let drop = resolve(&tx, drop)?.ok_or(Error::NotFound(drop))?;
        if keep == drop {
            let e = load(&tx, keep)?.ok_or(Error::NotFound(keep))?;
            return Ok(e);
        }
        let mut k = load(&tx, keep)?.ok_or(Error::NotFound(keep))?;
        let d = load(&tx, drop)?.ok_or(Error::NotFound(drop))?;
        if k.kind != d.kind {
            return Err(Error::KindMismatch { id: drop, existing: d.kind, given: k.kind });
        }
        k.absorb(&d);
        let t = now();
        k.updated_at = t;
        let (ks, ds) = (keep.to_string(), drop.to_string());
        tx.execute("UPDATE keys SET entity_id = ?1 WHERE entity_id = ?2", params![ks, ds])?;
        tx.execute("UPDATE refs SET entity_id = ?1 WHERE entity_id = ?2", params![ks, ds])?;
        tx.execute("UPDATE entities SET merged_into = ?1, deleted_at = ?2, updated_at = ?2 WHERE id = ?3", params![ks, t, ds])?;
        // earlier merges into `drop` now point straight at `keep`
        tx.execute("UPDATE entities SET merged_into = ?1 WHERE merged_into = ?2", params![ks, ds])?;
        store(&tx, &k, false)?;
        log(&tx, Op::Merge, drop, Some(keep), app, t)?;
        log(&tx, Op::Upsert, keep, None, app, t)?;
        tx.commit()?;
        Ok(k)
    }

    /// Deletes entity `id` (a tombstone stays for the change log). False if it wasn't there.
    pub fn delete(&mut self, app: &str, id: Uuid) -> Result<bool> {
        retry(|| self.delete_once(app, id))
    }

    fn delete_once(&mut self, app: &str, id: Uuid) -> Result<bool> {
        check_app(app)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(id) = resolve(&tx, id)? else { return Ok(false) };
        let t = now();
        let s = id.to_string();
        tx.execute("UPDATE entities SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2", params![t, s])?;
        tx.execute("DELETE FROM keys WHERE entity_id = ?1", [&s])?;
        tx.execute("DELETE FROM refs WHERE entity_id = ?1", [&s])?;
        log(&tx, Op::Delete, id, None, app, t)?;
        tx.commit()?;
        Ok(true)
    }

    /// Live cards, by name.
    pub fn list(&self, kind: Option<Kind>, limit: usize, offset: usize) -> Result<Vec<Entity>> {
        let mut st = self.conn.prepare("SELECT card FROM entities WHERE deleted_at IS NULL AND (?1 IS NULL OR kind = ?1) ORDER BY name COLLATE NOCASE, id LIMIT ?2 OFFSET ?3")?;
        let rows = st.query_map(params![kind.map(Kind::key), limit as i64, offset as i64], |r| r.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).map(|s| Entity::from_json(&s).map_err(Error::from)).collect()
    }

    /// Cards whose name, aliases, ids, emails, hostnames… contain every word of `query`
    /// (accents and case ignored).
    pub fn search(&self, query: &str, kind: Option<Kind>, limit: usize) -> Result<Vec<Entity>> {
        let words: Vec<String> = fold(query).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
        if words.is_empty() {
            return self.list(kind, limit, 0);
        }
        let mut st = self.conn.prepare("SELECT search, card FROM entities WHERE deleted_at IS NULL AND (?1 IS NULL OR kind = ?1) AND instr(search, ?2) > 0 ORDER BY name COLLATE NOCASE, id")?;
        let rows = st.query_map(params![kind.map(Kind::key), words[0]], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (search, card) = row?;
            if words.iter().all(|w| search.contains(w.as_str())) {
                out.push(Entity::from_json(&card)?);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Live cards per kind.
    pub fn counts(&self) -> Result<BTreeMap<Kind, usize>> {
        let mut st = self.conn.prepare("SELECT kind, count(*) FROM entities WHERE deleted_at IS NULL GROUP BY kind")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows.filter_map(|r| r.ok()).filter_map(|(k, n)| Some((Kind::parse(&k)?, n as usize))).collect())
    }

    /// Pairs of live cards of the same kind that share a name or alias (accents / case /
    /// punctuation ignored): merge candidates for the user to confirm. Ordered, each pair once.
    pub fn duplicates(&self, limit: usize) -> Result<Vec<(Entity, Entity)>> {
        let all = self.list(None, usize::MAX >> 1, 0)?;
        let mut by: BTreeMap<(Kind, String), Vec<usize>> = BTreeMap::new();
        for (i, e) in all.iter().enumerate() {
            let mut names: Vec<String> = std::iter::once(&e.name).chain(e.aliases.iter()).map(|n| fold(n)).filter(|n| !n.is_empty()).collect();
            names.sort();
            names.dedup();
            for n in names {
                by.entry((e.kind, n)).or_default().push(i);
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for idx in by.values().filter(|v| v.len() > 1) {
            for a in 0..idx.len() {
                for b in a + 1..idx.len() {
                    let (x, y) = (&all[idx[a]], &all[idx[b]]);
                    // two different tax ids / plates / MACs: same name, different things
                    let (kx, ky) = (x.natural_keys(), y.natural_keys());
                    let disjoint_keys = !kx.is_empty() && !ky.is_empty() && kx.iter().all(|k| !ky.contains(k)) && kx.iter().any(|k| ky.iter().any(|j| prefix(j) == prefix(k)));
                    if disjoint_keys {
                        continue;
                    }
                    let key = if x.id < y.id { (x.id, y.id) } else { (y.id, x.id) };
                    if seen.insert(key) {
                        out.push((x.clone(), y.clone()));
                        if out.len() >= limit {
                            return Ok(out);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// The newest change's `seq` (0 for an empty log).
    pub fn last_seq(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT coalesce(max(seq), 0) FROM changes", [], |r| r.get(0))?)
    }

    /// Changes after `seq`, oldest first.
    pub fn changes_since(&self, seq: i64, limit: usize) -> Result<Vec<Change>> {
        let mut st = self.conn.prepare("SELECT seq, op, entity_id, into_id, app, at FROM changes WHERE seq > ?1 ORDER BY seq LIMIT ?2")?;
        let rows = st.query_map(params![seq, limit as i64], |r| {
            Ok(Change { seq: r.get(0)?, op: Op::parse(&r.get::<_, String>(1)?), id: uuid(&r.get::<_, String>(2)?), into: r.get::<_, Option<String>>(3)?.map(|s| uuid(&s)), app: r.get(4)?, at: r.get(5)? })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Where `app`'s sync stands (0: never synced).
    pub fn cursor(&self, app: &str) -> Result<i64> {
        Ok(self.conn.query_row("SELECT seq FROM cursors WHERE app = ?1", [app], |r| r.get(0)).optional()?.unwrap_or(0))
    }

    /// What other apps changed since `app`'s cursor, at most `limit` log entries, collapsed to
    /// one item per entity (its latest change). `app`'s own changes are skipped, unless another
    /// app touched the same entity in the batch.
    pub fn pull(&self, app: &str, limit: usize) -> Result<Pull> {
        check_app(app)?;
        let from = self.cursor(app)?;
        let changes = self.changes_since(from, limit.max(1) + 1)?;
        let more = changes.len() > limit.max(1);
        let changes: Vec<Change> = changes.into_iter().take(limit.max(1)).collect();
        let cursor = changes.last().map(|c| c.seq).unwrap_or(from);
        let mut latest: BTreeMap<Uuid, (Change, bool)> = BTreeMap::new();
        for c in changes {
            let foreign = c.app != app;
            let e = latest.entry(c.id).or_insert((c.clone(), false));
            e.0 = c;
            e.1 |= foreign;
        }
        let mut items: Vec<Synced> = Vec::new();
        for (_, (change, foreign)) in latest {
            if !foreign {
                continue;
            }
            let entity = match change.op {
                Op::Upsert => load_live(&self.conn, change.id)?,
                Op::Delete | Op::Merge => None,
            };
            items.push(Synced { change, entity });
        }
        items.sort_by_key(|s| s.change.seq);
        Ok(Pull { items, cursor, more })
    }

    /// `app` has applied everything up to `seq`.
    pub fn ack(&mut self, app: &str, seq: i64) -> Result<()> {
        check_app(app)?;
        retry(|| self.ack_once(app, seq))
    }

    fn ack_once(&mut self, app: &str, seq: i64) -> Result<()> {
        self.conn.execute("INSERT INTO cursors(app, seq, at) VALUES (?1, ?2, ?3) ON CONFLICT(app) DO UPDATE SET seq = max(seq, excluded.seq), at = excluded.at", params![app, seq, now()])?;
        Ok(())
    }

    /// Apps holding a ref to entity `id` (from the refs table, so merged-in refs count).
    pub fn referencing_apps(&self, id: Uuid) -> Result<Vec<(String, String)>> {
        let Some(id) = self.resolve(id)? else { return Ok(vec![]) };
        let mut st = self.conn.prepare("SELECT app, ref FROM refs WHERE entity_id = ?1 ORDER BY app, ref")?;
        let rows = st.query_map([id.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

fn busy(e: &Error) -> bool {
    matches!(e, Error::Sql(rusqlite::Error::SqliteFailure(f, _)) if matches!(f.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
}

/// SQLite's busy handler waits up to [`BUSY_TIMEOUT`], but polling waiters can starve behind a
/// stream of other writers (seen on Windows), and some lock upgrades return `SQLITE_BUSY` without
/// waiting at all. A transaction that failed that way changed nothing, so run it again.
fn retry<T>(mut f: impl FnMut() -> Result<T>) -> Result<T> {
    let mut attempt: u64 = 0;
    loop {
        match f() {
            Err(e) if busy(&e) && attempt < 8 => {
                attempt += 1;
                let jitter = (std::process::id() as u64 ^ chrono::Utc::now().timestamp_subsec_nanos() as u64) % 50;
                std::thread::sleep(Duration::from_millis(20 * attempt + jitter));
            }
            r => return r,
        }
    }
}

fn prefix(k: &str) -> &str {
    k.rsplit_once(':').map(|(p, _)| p).unwrap_or(k)
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let app_id: i32 = tx.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let has_tables: i64 = tx.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table'", [], |r| r.get(0))?;
    if app_id != APPLICATION_ID && (app_id != 0 || has_tables > 0) {
        return Err(Error::NotOurs);
    }
    if version > MIGRATIONS.len() as i64 {
        return Err(Error::NewerDatabase(version));
    }
    for (i, m) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        tx.execute_batch(m)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
    }
    if app_id == 0 {
        tx.pragma_update(None, "application_id", APPLICATION_ID)?;
    }
    tx.commit()?;
    Ok(())
}

fn query_strings(conn: &Connection, sql: &str, arg: &str) -> Result<Vec<String>> {
    let mut st = conn.prepare(sql)?;
    let rows = st.query_map([arg], |r| r.get::<_, String>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn exists_any(conn: &Connection, id: Uuid) -> Result<bool> {
    Ok(conn.query_row("SELECT 1 FROM entities WHERE id = ?1", [id.to_string()], |_| Ok(())).optional()?.is_some())
}

fn resolve(conn: &Connection, id: Uuid) -> Result<Option<Uuid>> {
    let mut cur = id;
    for _ in 0..32 {
        let row: Option<(Option<i64>, Option<String>)> = conn.query_row("SELECT deleted_at, merged_into FROM entities WHERE id = ?1", [cur.to_string()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        match row {
            None => return Ok(None),
            Some((_, Some(into))) => cur = uuid(&into),
            Some((Some(_), None)) => return Ok(None),
            Some((None, None)) => return Ok(Some(cur)),
        }
    }
    Ok(None)
}

fn load(conn: &Connection, id: Uuid) -> Result<Option<Entity>> {
    let card: Option<String> = conn.query_row("SELECT card FROM entities WHERE id = ?1", [id.to_string()], |r| r.get(0)).optional()?;
    card.map(|c| Entity::from_json(&c).map_err(Error::from)).transpose()
}

fn load_live(conn: &Connection, id: Uuid) -> Result<Option<Entity>> {
    let card: Option<String> = conn.query_row("SELECT card FROM entities WHERE id = ?1 AND deleted_at IS NULL", [id.to_string()], |r| r.get(0)).optional()?;
    card.map(|c| Entity::from_json(&c).map_err(Error::from)).transpose()
}

fn find_existing(conn: &Connection, app: &str, card: &Entity) -> Result<Option<(Uuid, Matched)>> {
    let same_kind = |id: Uuid| -> Result<bool> { Ok(load(conn, id)?.is_some_and(|e| e.kind == card.kind)) };
    if !card.id.is_nil()
        && let Some(id) = resolve(conn, card.id)?
    {
        return Ok(Some((id, Matched::Id)));
    }
    let by_ref = |a: &str, r: &str| -> Result<Option<Uuid>> {
        let id: Option<String> = conn.query_row("SELECT entity_id FROM refs WHERE app = ?1 AND ref = ?2", params![a, r], |row| row.get(0)).optional()?;
        match id {
            Some(id) => resolve(conn, uuid(&id)),
            None => Ok(None),
        }
    };
    if let Some(r) = card.refs.get(app)
        && let Some(id) = by_ref(app, r)?
        && same_kind(id)?
    {
        return Ok(Some((id, Matched::OwnRef)));
    }
    for (a, r) in card.refs.iter().filter(|(a, _)| a.as_str() != app) {
        if let Some(id) = by_ref(a, r)?
            && same_kind(id)?
        {
            return Ok(Some((id, Matched::OtherRef)));
        }
    }
    for k in card.natural_keys() {
        let id: Option<String> = conn.query_row("SELECT entity_id FROM keys WHERE key = ?1", [&k], |row| row.get(0)).optional()?;
        if let Some(id) = id
            && let Some(id) = resolve(conn, uuid(&id))?
            && same_kind(id)?
        {
            return Ok(Some((id, Matched::Key)));
        }
    }
    Ok(None)
}

fn store(conn: &Connection, e: &Entity, insert: bool) -> Result<()> {
    let id = e.id.to_string();
    let json = e.to_json();
    if insert {
        conn.execute(
            "INSERT INTO entities(id, kind, name, search, card, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, e.kind.key(), e.name, e.search_text(), json, e.created_at, e.updated_at],
        )?;
    } else {
        conn.execute("UPDATE entities SET kind = ?2, name = ?3, search = ?4, card = ?5, updated_at = ?6 WHERE id = ?1", params![id, e.kind.key(), e.name, e.search_text(), json, e.updated_at])?;
    }
    // a key or ref already held by another card stays with it (first come); duplicates() and
    // merge() are how the user reconciles them
    for k in e.natural_keys() {
        conn.execute("INSERT OR IGNORE INTO keys(key, entity_id) VALUES (?1, ?2)", params![k, id])?;
    }
    for (a, r) in &e.refs {
        conn.execute("INSERT OR IGNORE INTO refs(app, ref, entity_id) VALUES (?1, ?2, ?3)", params![a, r, id])?;
    }
    Ok(())
}

fn log(conn: &Connection, op: Op, id: Uuid, into: Option<Uuid>, app: &str, at: i64) -> Result<()> {
    conn.execute("INSERT INTO changes(entity_id, op, into_id, app, at) VALUES (?1, ?2, ?3, ?4, ?5)", params![id.to_string(), op.key(), into.map(|u| u.to_string()), app, at])?;
    Ok(())
}
