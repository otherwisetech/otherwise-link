//! Entity cards: the people, companies, vehicles, properties, devices, sites and digital assets
//! that several apps talk about. Schema version [`SCHEMA`]; the JSON is specified in
//! `docs/SPEC.md`.
//!
//! ```json
//! { "schema": 1, "id": "6f0c…", "kind": "company", "name": "Otherwise Technologies LLC",
//!   "aliases": ["Otherwise"], "refs": { "vigia": "entity-12", "expense": "vendor-9" },
//!   "company": { "taxIds": [ { "country": "US", "scheme": "EIN", "value": "123456789" } ],
//!                "country": "US" } }
//! ```
//!
//! - `id` is a UUID assigned once (v4) and never changed; a merged-away card keeps resolving to
//!   the survivor through the registry. A nil / missing id means "new".
//! - `refs` maps an app id to that app's own id for the same thing, so an app can find "its"
//!   card and others can deep-link back.
//! - Exactly one kind section (`person`, `company`, …) may be present and it must match `kind`.
//! - `ext` holds app-specific extras under the app's id (`ext.nodo = {…}`); others keep them as is.
//! - Readers ignore unknown fields, so newer writers can add some without a schema bump. A
//!   breaking change bumps `schema`; readers refuse cards from a newer major schema.
use crate::tax::TaxId;
use crate::text::{compact_upper, fold};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Person,
    Company,
    Vehicle,
    Property,
    Device,
    Site,
    Digital,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Person, Kind::Company, Kind::Vehicle, Kind::Property, Kind::Device, Kind::Site, Kind::Digital];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Person => "person",
            Kind::Company => "company",
            Kind::Vehicle => "vehicle",
            Kind::Property => "property",
            Kind::Device => "device",
            Kind::Site => "site",
            Kind::Digital => "digital",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.key() == s)
    }
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Person {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub given_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family_name: Option<String>,
    /// `YYYY-MM-DD`, or just `YYYY` / `YYYY-MM` when that is all that is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub death_date: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub phones: Vec<String>,
    /// National ids (CC, SSN, CPF…) and tax ids.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tax_ids: Vec<TaxId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Company {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_name: Option<String>,
    /// One per country / scheme: NIT (CO), EIN (US), CNPJ (BR), SIREN (FR), RFC (MX)…
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tax_ids: Vec<TaxId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub phones: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Vehicle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub make: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Property {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// Cadastral / parcel number (número predial, APN…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cadastral_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lon: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Device {
    /// `aa:bb:cc:dd:ee:ff`, lower case (normalized on save).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub macs: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hostnames: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ips: Vec<String>,
    /// Tailscale node id (`nXXXX…` / stable id from `tailscale status --json`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tailscale_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Site {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lon: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius_m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Digital {
    /// A domain name, lower case, no scheme (`otherwisetech.com`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Service the account is on (`github`, `aws`, `google`…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// Account / user name / account number on that service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entity {
    #[serde(default = "schema_default")]
    pub schema: u32,
    #[serde(default, skip_serializing_if = "Uuid::is_nil")]
    pub id: Uuid,
    pub kind: Kind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// app id → that app's id for this entity.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<Person>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company: Option<Company>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vehicle: Option<Vehicle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property: Option<Property>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<Device>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<Site>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digital: Option<Digital>,
    /// app id → anything that app wants to keep with the card.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
    /// Unix seconds; set by the registry.
    #[serde(default, skip_serializing_if = "is_default")]
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub updated_at: i64,
}

fn schema_default() -> u32 {
    SCHEMA
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardError {
    Json(String),
    NewerSchema(u32),
    EmptyName,
    WrongSection { kind: Kind, section: &'static str },
    BadValue { field: &'static str, value: String },
}

impl std::fmt::Display for CardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CardError::Json(e) => write!(f, "invalid card: {e}"),
            CardError::NewerSchema(n) => write!(f, "card schema {n} is newer than this app understands ({SCHEMA})"),
            CardError::EmptyName => write!(f, "a card needs a name"),
            CardError::WrongSection { kind, section } => write!(f, "a {} card can't have a {section} section", kind.key()),
            CardError::BadValue { field, value } => write!(f, "invalid {field}: {value:?}"),
        }
    }
}

impl std::error::Error for CardError {}

/// `aa:bb:cc:dd:ee:ff` from any common spelling (`AA-BB-…`, `aabb.ccdd.eeff`, `aabbccddeeff`).
pub fn normalize_mac(s: &str) -> Option<String> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_ascii_lowercase();
    let seps_ok = s.chars().all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '-' | '.'));
    if hex.len() != 12 || !seps_ok {
        return None;
    }
    Some(hex.as_bytes().chunks(2).map(|p| std::str::from_utf8(p).unwrap_or("00")).collect::<Vec<_>>().join(":"))
}

pub fn normalize_domain(s: &str) -> String {
    let s = s.trim().to_ascii_lowercase();
    let s = s.split("://").last().unwrap_or("").to_string();
    let s = s.split(['/', '?', '#']).next().unwrap_or("");
    s.strip_prefix("www.").unwrap_or(s).trim_end_matches('.').to_string()
}

fn lat_lon_ok(lat: Option<f64>, lon: Option<f64>) -> Result<(), CardError> {
    if let Some(l) = lat
        && !(-90.0..=90.0).contains(&l)
    {
        return Err(CardError::BadValue { field: "lat", value: l.to_string() });
    }
    if let Some(l) = lon
        && !(-180.0..=180.0).contains(&l)
    {
        return Err(CardError::BadValue { field: "lon", value: l.to_string() });
    }
    Ok(())
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !s.is_empty() && !v.contains(&s) {
        v.push(s);
    }
}

impl Entity {
    pub fn new(kind: Kind, name: impl Into<String>) -> Entity {
        Entity {
            schema: SCHEMA,
            id: Uuid::nil(),
            kind,
            name: name.into(),
            aliases: vec![],
            refs: BTreeMap::new(),
            tags: vec![],
            notes: None,
            person: None,
            company: None,
            vehicle: None,
            property: None,
            device: None,
            site: None,
            digital: None,
            ext: BTreeMap::new(),
            created_at: 0,
            updated_at: 0,
        }
    }

    pub fn from_json(s: &str) -> Result<Entity, CardError> {
        let v: Value = serde_json::from_str(s).map_err(|e| CardError::Json(e.to_string()))?;
        Entity::from_value(v)
    }

    pub fn from_value(v: Value) -> Result<Entity, CardError> {
        if let Some(n) = v.get("schema").and_then(Value::as_u64)
            && n as u32 > SCHEMA
        {
            return Err(CardError::NewerSchema(n as u32));
        }
        serde_json::from_value(v).map_err(|e| CardError::Json(e.to_string()))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn with_ref(mut self, app: &str, id: impl Into<String>) -> Entity {
        self.refs.insert(app.to_string(), id.into());
        self
    }

    pub fn with_alias(mut self, a: impl Into<String>) -> Entity {
        push_unique(&mut self.aliases, a.into());
        self
    }

    /// The kind sections present, by name.
    fn sections(&self) -> Vec<(&'static str, Kind)> {
        let mut v = Vec::new();
        if self.person.is_some() {
            v.push(("person", Kind::Person));
        }
        if self.company.is_some() {
            v.push(("company", Kind::Company));
        }
        if self.vehicle.is_some() {
            v.push(("vehicle", Kind::Vehicle));
        }
        if self.property.is_some() {
            v.push(("property", Kind::Property));
        }
        if self.device.is_some() {
            v.push(("device", Kind::Device));
        }
        if self.site.is_some() {
            v.push(("site", Kind::Site));
        }
        if self.digital.is_some() {
            v.push(("digital", Kind::Digital));
        }
        v
    }

    /// Checks the card and normalizes ids in place (tax ids, MACs, plates, domains, aliases).
    pub fn normalize(&mut self) -> Result<(), CardError> {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            return Err(CardError::EmptyName);
        }
        if self.schema == 0 || self.schema > SCHEMA {
            self.schema = SCHEMA;
        }
        if let Some((section, _)) = self.sections().into_iter().find(|(_, k)| *k != self.kind) {
            return Err(CardError::WrongSection { kind: self.kind, section });
        }
        for app in self.refs.keys() {
            if !crate::link::valid_app_id(app) {
                return Err(CardError::BadValue { field: "refs", value: app.clone() });
            }
        }
        let mut aliases = Vec::new();
        for a in self.aliases.drain(..) {
            let a = a.trim().to_string();
            if a != self.name {
                push_unique(&mut aliases, a);
            }
        }
        self.aliases = aliases;
        let tax = |v: &mut Vec<TaxId>| {
            let mut out: Vec<TaxId> = Vec::new();
            for t in v.drain(..) {
                let t = t.normalized();
                if !t.value.is_empty() && !out.contains(&t) {
                    out.push(t);
                }
            }
            *v = out;
        };
        if let Some(p) = &mut self.person {
            tax(&mut p.tax_ids);
            p.emails = p.emails.iter().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
        }
        if let Some(c) = &mut self.company {
            tax(&mut c.tax_ids);
            c.emails = c.emails.iter().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
        }
        if let Some(v) = &mut self.vehicle {
            v.plate = v.plate.as_deref().map(compact_upper).filter(|p| !p.is_empty());
            v.vin = v.vin.as_deref().map(compact_upper).filter(|p| !p.is_empty());
            v.country = v.country.as_deref().map(|c| c.trim().to_ascii_uppercase());
        }
        if let Some(d) = &mut self.device {
            let mut macs = Vec::new();
            for m in &d.macs {
                push_unique(&mut macs, normalize_mac(m).ok_or_else(|| CardError::BadValue { field: "mac", value: m.clone() })?);
            }
            d.macs = macs;
            d.hostnames = d.hostnames.iter().map(|h| h.trim().trim_end_matches('.').to_lowercase()).filter(|h| !h.is_empty()).collect();
        }
        if let Some(p) = &self.property {
            lat_lon_ok(p.lat, p.lon)?;
        }
        if let Some(s) = &self.site {
            lat_lon_ok(s.lat, s.lon)?;
        }
        if let Some(d) = &mut self.digital {
            d.domain = d.domain.as_deref().map(normalize_domain).filter(|d| !d.is_empty());
            d.service = d.service.as_deref().map(|s| s.trim().to_lowercase());
        }
        Ok(())
    }

    /// Identifiers that name this thing unambiguously across apps: two cards sharing one are the
    /// same entity. `tax:CO:NIT:…`, `email:…`, `plate:CO:ABC123`, `vin:…`, `cadastral:CO:…`,
    /// `mac:aa:bb:…`, `tailscale:…`, `domain:…`, `account:github:…`. Call after
    /// [`Entity::normalize`].
    pub fn natural_keys(&self) -> Vec<String> {
        let mut k = Vec::new();
        if let Some(p) = &self.person {
            k.extend(p.tax_ids.iter().map(TaxId::key));
            k.extend(p.emails.iter().map(|e| format!("email:{e}")));
        }
        if let Some(c) = &self.company {
            k.extend(c.tax_ids.iter().map(TaxId::key));
        }
        if let Some(v) = &self.vehicle {
            if let Some(p) = &v.plate {
                k.push(format!("plate:{}:{p}", v.country.as_deref().unwrap_or("")));
            }
            if let Some(vin) = &v.vin {
                k.push(format!("vin:{vin}"));
            }
        }
        if let Some(p) = &self.property
            && let Some(c) = &p.cadastral_id
        {
            k.push(format!("cadastral:{}:{}", p.country.as_deref().unwrap_or("").to_ascii_uppercase(), compact_upper(c)));
        }
        if let Some(d) = &self.device {
            k.extend(d.macs.iter().map(|m| format!("mac:{m}")));
            if let Some(t) = &d.tailscale_id {
                k.push(format!("tailscale:{}", t.trim()));
            }
        }
        if let Some(d) = &self.digital {
            if let (Some(s), Some(a)) = (&d.service, &d.account) {
                k.push(format!("account:{s}:{}", a.trim().to_lowercase()));
            } else if let Some(dom) = &d.domain {
                k.push(format!("domain:{dom}"));
            }
        }
        k.sort();
        k.dedup();
        k
    }

    /// Everything a search should match: name, aliases, keys, emails, hostnames…, folded.
    pub fn search_text(&self) -> String {
        let mut parts: Vec<String> = vec![self.name.clone()];
        parts.extend(self.aliases.iter().cloned());
        parts.extend(self.natural_keys().into_iter().map(|k| k.split(':').skip(1).collect::<Vec<_>>().join(" ")));
        parts.extend(self.tags.iter().cloned());
        if let Some(c) = &self.company {
            parts.extend(c.legal_name.clone());
        }
        if let Some(d) = &self.device {
            parts.extend(d.hostnames.iter().cloned());
            parts.extend(d.ips.iter().cloned());
        }
        if let Some(d) = &self.digital {
            parts.extend(d.domain.clone());
        }
        parts.iter().map(|p| fold(p)).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" | ")
    }

    /// `other` folded into `self`: `self`'s scalars win where set, `other` fills the gaps; lists,
    /// refs, tags and ext are united; `other`'s name becomes an alias if it differs.
    pub fn absorb(&mut self, other: &Entity) {
        let mut mine = serde_json::to_value(&*self).unwrap_or(Value::Null);
        let theirs = serde_json::to_value(other).unwrap_or(Value::Null);
        fill(&mut mine, &theirs);
        let (id, kind, name, created) = (self.id, self.kind, self.name.clone(), self.created_at.min(if other.created_at > 0 { other.created_at } else { self.created_at }));
        if let Ok(mut e) = serde_json::from_value::<Entity>(mine) {
            e.id = id;
            e.kind = kind;
            e.name = name;
            e.created_at = if created > 0 { created } else { self.created_at };
            *self = e;
        }
        if other.name.trim() != self.name {
            push_unique(&mut self.aliases, other.name.trim().to_string());
        }
        for a in &other.aliases {
            if a != &self.name {
                push_unique(&mut self.aliases, a.clone());
            }
        }
    }

    /// `patch` applied over `self` (an update from an app): `patch`'s scalars win where set,
    /// lists are united. Like [`Entity::absorb`] with the roles swapped, keeping `self`'s id.
    pub fn apply(&mut self, patch: &Entity) {
        let mut merged = patch.clone();
        merged.id = self.id;
        merged.created_at = self.created_at;
        let old_name = self.name.clone();
        merged.absorb(self);
        // the previous name is not an alias of the renamed card unless someone said so
        if merged.name != old_name && !self.aliases.contains(&old_name) && !patch.aliases.contains(&old_name) {
            merged.aliases.retain(|a| a != &old_name);
        }
        *self = merged;
    }
}

/// Fills `dst`'s missing / null fields from `src`; arrays are united (order kept); objects merge
/// recursively.
fn fill(dst: &mut Value, src: &Value) {
    match (dst, src) {
        (Value::Object(d), Value::Object(s)) => {
            for (k, v) in s {
                match d.get_mut(k) {
                    Some(Value::Null) | None => {
                        d.insert(k.clone(), v.clone());
                    }
                    Some(existing) => fill(existing, v),
                }
            }
        }
        (Value::Array(d), Value::Array(s)) => {
            for v in s {
                if !d.contains(v) {
                    d.push(v.clone());
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_and_domain() {
        assert_eq!(normalize_mac("AA-BB-CC-DD-EE-0F").as_deref(), Some("aa:bb:cc:dd:ee:0f"));
        assert_eq!(normalize_mac("aabb.ccdd.ee0f").as_deref(), Some("aa:bb:cc:dd:ee:0f"));
        assert_eq!(normalize_mac("aa:bb:cc"), None);
        assert_eq!(normalize_mac("zz:bb:cc:dd:ee:ff:00"), None);
        assert_eq!(normalize_domain("https://www.OtherwiseTech.com/apps?x"), "otherwisetech.com");
    }
}
