use otherwise_link::link::{Link, LinkError, app_scheme, decode, encode, valid_app_id};

#[test]
fn round_trips() {
    let cases = [
        Link::new("vigia").segment("item").segment("42"),
        Link::new("nodo").segment("device").segment("6f0c2c1e-0000-4000-8000-000000000001").param("map", "home"),
        Link::new("expense").segment("receipt").segment("a b/c ñ ✓").param("q", "x=y&z").param("q", "2").param("empty", ""),
        Link::new("otherwise-apps").segment("today"),
        Link::new("rama"),
        Link::new("diskmap").param("path", "/Users/me/Down loads"),
    ];
    for l in cases {
        let url = l.to_url();
        assert_eq!(Link::parse(&url).unwrap(), l, "{url}");
        let app = l.app_url();
        assert_eq!(Link::parse(&app).unwrap(), l, "{app}");
        assert_eq!(url.parse::<Link>().unwrap().to_string(), url);
    }
}

#[test]
fn forms() {
    let l = Link::parse("otherwise://vigia/item/42?tab=docs#frag").unwrap();
    assert_eq!(l.app, "vigia");
    assert_eq!(l.path, ["item", "42"]);
    assert_eq!(l.get("tab"), Some("docs"));
    assert_eq!(l.at(1), Some("42"));
    assert_eq!(l.to_url(), "otherwise://vigia/item/42?tab=docs");
    assert_eq!(l.app_url(), "otherwise-vigia://item/42?tab=docs");
    // per-app scheme, upper-case scheme / host, doubled slashes, plus as space
    assert_eq!(Link::parse("Otherwise-Vigia://item//42/").unwrap(), Link::new("vigia").segment("item").segment("42"));
    assert_eq!(Link::parse("OTHERWISE://Vigia/item").unwrap().app, "vigia");
    assert_eq!(Link::parse("otherwise://nodo/search?q=a+b%2Bc").unwrap().get("q"), Some("a b+c"));
    assert_eq!(Link::parse("otherwise://nodo?x").unwrap().get("x"), Some(""));
    // the launcher's own links have no per-app form
    assert_eq!(Link::new("otherwise-apps").segment("today").app_url(), "otherwise://otherwise-apps/today");
    assert_eq!(app_scheme("nodo"), "otherwise-nodo");
}

#[test]
fn rejects() {
    assert_eq!(Link::parse("https://otherwisetech.com"), Err(LinkError::Scheme("https".into())));
    assert_eq!(Link::parse("vigia/item"), Err(LinkError::NotALink));
    assert!(matches!(Link::parse("otherwise://"), Err(LinkError::App(_))));
    assert!(matches!(Link::parse("otherwise://bad_app/x"), Err(LinkError::App(_))));
    assert!(matches!(Link::parse("otherwise://-x/x"), Err(LinkError::App(_))));
    assert_eq!(Link::parse("otherwise://vigia/%zz"), Err(LinkError::Encoding));
    assert_eq!(Link::parse("otherwise://vigia/%ff"), Err(LinkError::Encoding));
    assert_eq!(Link::parse("otherwise://vigia/%4"), Err(LinkError::Encoding));
    assert!(!valid_app_id(""));
    assert!(!valid_app_id(&"a".repeat(65)));
    assert!(valid_app_id("genealogy"));
}

#[test]
fn from_args_finds_the_link() {
    let args = ["/usr/bin/vigia-app", "--profile", "otherwise-vigia://item/7"];
    assert_eq!(Link::from_args(args).unwrap().path, ["item", "7"]);
    assert_eq!(Link::from_args(["app", "file.pdf"]), None);
}

#[test]
fn percent_encoding() {
    assert_eq!(encode("a b/ñ~._-"), "a%20b%2F%C3%B1~._-");
    assert_eq!(decode("a%20b%2F%C3%B1", false).unwrap(), "a b/ñ");
    assert_eq!(decode("a+b", false).unwrap(), "a+b");
}
