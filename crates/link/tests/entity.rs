use otherwise_link::entity::*;
use otherwise_link::tax::TaxId;
use serde_json::json;

#[test]
fn json_shape_round_trips() {
    let src = json!({
        "schema": 1, "id": "6f0c2c1e-0000-4000-8000-000000000001", "kind": "company", "name": "Otherwise Technologies LLC",
        "aliases": ["Otherwise"], "refs": {"vigia": "entity-12", "expense": "vendor-9"},
        "company": {"taxIds": [{"country": "US", "scheme": "EIN", "value": "123456789"}], "country": "US"},
        "ext": {"vigia": {"color": "teal"}}, "futureField": 1
    });
    let e = Entity::from_value(src.clone()).unwrap();
    assert_eq!(e.kind, Kind::Company);
    assert_eq!(e.refs["vigia"], "entity-12");
    assert_eq!(e.company.as_ref().unwrap().tax_ids[0].scheme, "EIN");
    let back: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
    let mut expected = src;
    expected.as_object_mut().unwrap().remove("futureField");
    assert_eq!(back, expected);
    assert_eq!(Entity::from_json(&e.to_json()).unwrap(), e);
}

#[test]
fn every_kind_serializes() {
    for k in Kind::ALL {
        let e = Entity::new(k, "x");
        let v: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["kind"], k.key());
        assert_eq!(Kind::parse(k.key()), Some(k));
        assert!(v.get("id").is_none(), "a new card has no id yet");
    }
}

#[test]
fn newer_schema_is_refused() {
    assert_eq!(Entity::from_value(json!({"schema": 2, "kind": "person", "name": "A"})), Err(CardError::NewerSchema(2)));
    assert!(matches!(Entity::from_json("{"), Err(CardError::Json(_))));
    assert!(matches!(Entity::from_value(json!({"kind": "planet", "name": "A"})), Err(CardError::Json(_))));
}

#[test]
fn normalize_and_keys() {
    let mut e = Entity::new(Kind::Device, " NAS ").with_alias("NAS").with_alias("storage");
    e.device = Some(Device { macs: vec!["AA-BB-CC-DD-EE-FF".into(), "aabb.ccdd.eeff".into()], hostnames: vec!["NAS.local.".into()], tailscale_id: Some("nABC".into()), ..Default::default() });
    e.normalize().unwrap();
    assert_eq!(e.name, "NAS");
    assert_eq!(e.aliases, ["storage"]);
    assert_eq!(e.device.as_ref().unwrap().macs, ["aa:bb:cc:dd:ee:ff"]);
    assert_eq!(e.device.as_ref().unwrap().hostnames, ["nas.local"]);
    assert_eq!(e.natural_keys(), ["mac:aa:bb:cc:dd:ee:ff", "tailscale:nABC"]);
    assert!(e.search_text().contains("nas local"));

    let mut v = Entity::new(Kind::Vehicle, "Mazda");
    v.vehicle = Some(Vehicle { plate: Some("abc-123".into()), country: Some("co".into()), vin: Some("1hgcm82633a004352".into()), ..Default::default() });
    v.normalize().unwrap();
    assert_eq!(v.natural_keys(), ["plate:CO:ABC123", "vin:1HGCM82633A004352"]);

    let mut c = Entity::new(Kind::Company, "ACME");
    c.company = Some(Company { tax_ids: vec![TaxId::new("co", "nit", "800.197.268-4"), TaxId::new("CO", "NIT", "8001972684")], ..Default::default() });
    c.normalize().unwrap();
    assert_eq!(c.company.as_ref().unwrap().tax_ids.len(), 1);
    assert_eq!(c.natural_keys(), ["tax:CO:NIT:8001972684"]);

    let mut d = Entity::new(Kind::Digital, "site");
    d.digital = Some(Digital { domain: Some("https://WWW.Example.com/".into()), ..Default::default() });
    d.normalize().unwrap();
    assert_eq!(d.natural_keys(), ["domain:example.com"]);
}

#[test]
fn invalid_cards() {
    assert_eq!(Entity::new(Kind::Person, "  ").normalize(), Err(CardError::EmptyName));
    let mut e = Entity::new(Kind::Person, "Ana");
    e.company = Some(Company::default());
    assert_eq!(e.normalize(), Err(CardError::WrongSection { kind: Kind::Person, section: "company" }));
    let mut s = Entity::new(Kind::Site, "Pole");
    s.site = Some(Site { lat: Some(91.0), lon: Some(0.0), ..Default::default() });
    assert!(matches!(s.normalize(), Err(CardError::BadValue { field: "lat", .. })));
    let mut m = Entity::new(Kind::Device, "x");
    m.device = Some(Device { macs: vec!["nope".into()], ..Default::default() });
    assert!(matches!(m.normalize(), Err(CardError::BadValue { field: "mac", .. })));
    let r = Entity::new(Kind::Person, "x").with_ref("Bad App", "1");
    assert!(matches!(r.clone().normalize(), Err(CardError::BadValue { field: "refs", .. })));
}

#[test]
fn absorb_fills_gaps_and_unites() {
    let mut a = Entity::new(Kind::Person, "Ana Pérez").with_ref("vigia", "p1");
    a.person = Some(Person { emails: vec!["ana@x.co".into()], birth_date: Some("1990-01-02".into()), ..Default::default() });
    let mut b = Entity::new(Kind::Person, "Ana P.").with_ref("expense", "c7").with_ref("vigia", "p9");
    b.person = Some(Person { emails: vec!["ana@y.co".into()], birth_date: Some("1991-01-01".into()), country: Some("CO".into()), ..Default::default() });
    a.absorb(&b);
    let p = a.person.as_ref().unwrap();
    assert_eq!(p.birth_date.as_deref(), Some("1990-01-02"), "the survivor's value wins");
    assert_eq!(p.country.as_deref(), Some("CO"), "gaps are filled");
    assert_eq!(p.emails, ["ana@x.co", "ana@y.co"]);
    assert_eq!(a.refs["vigia"], "p1");
    assert_eq!(a.refs["expense"], "c7");
    assert_eq!(a.aliases, ["Ana P."]);

    // apply: the patch wins, nothing is lost, a rename doesn't invent an alias
    let mut base = Entity::new(Kind::Person, "Old").with_alias("Nick");
    base.person = Some(Person { phones: vec!["1".into()], ..Default::default() });
    let mut patch = Entity::new(Kind::Person, "New");
    patch.person = Some(Person { phones: vec!["2".into()], ..Default::default() });
    base.apply(&patch);
    assert_eq!(base.name, "New");
    assert_eq!(base.aliases, ["Nick"]);
    assert_eq!(base.person.unwrap().phones, ["2", "1"]);
}
