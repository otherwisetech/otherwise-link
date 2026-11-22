//! Public holidays computed from their rules (no data files, no network), shared by every app so
//! Vigía, expense and Otherwise Apps' calendar agree: Colombia (Ley Emiliani Monday moves and the
//! Easter-based days), US federal, Wyoming state offices, Spain (national), Mexico (federal),
//! Brazil (national) and Canada (federal); plus business-day shifting.
//!
//! Each holiday has a stable [`Holiday::id`] (`co_new_year`, `us_thanksgiving`…) apps can label
//! in the user's language, and [`Holiday::name`], its proper name in the country's language.
use chrono::{Datelike, Duration, NaiveDate, Weekday};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Country {
    Co,
    Us,
    UsWy,
    Es,
    Mx,
    Br,
    Ca,
}

impl Country {
    pub const ALL: [Country; 7] = [Country::Co, Country::Us, Country::UsWy, Country::Es, Country::Mx, Country::Br, Country::Ca];

    /// `co`, `us`, `us-wy`, `es`, `mx`, `br`, `ca`.
    pub fn id(self) -> &'static str {
        match self {
            Country::Co => "co",
            Country::Us => "us",
            Country::UsWy => "us-wy",
            Country::Es => "es",
            Country::Mx => "mx",
            Country::Br => "br",
            Country::Ca => "ca",
        }
    }

    pub fn from_id(s: &str) -> Option<Country> {
        let s = s.trim().to_ascii_lowercase().replace('_', "-");
        Country::ALL.into_iter().find(|c| c.id() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holiday {
    pub date: NaiveDate,
    /// Stable key: `<country>_<name>` (`co_new_year`, `us_wy_equality_day`).
    pub id: &'static str,
    /// The proper name in the country's own language.
    pub name: &'static str,
    pub country: Country,
}

/// Easter Sunday (Gregorian; anonymous / Meeus algorithm).
pub fn easter(y: i32) -> NaiveDate {
    let a = y % 19;
    let b = y / 100;
    let c = y % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    ymd(y, month as u32, day as u32)
}

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap_or_default()
}

fn add(x: NaiveDate, n: i64) -> NaiveDate {
    x + Duration::days(n)
}

fn last_day(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    add(ymd(ny, nm, 1), -1).day()
}

/// The `n`th `wd` of the month (n = 1…5), or the last one with `n = -1`.
fn nth(y: i32, m: u32, wd: Weekday, n: i32) -> NaiveDate {
    if n < 0 {
        let mut x = ymd(y, m, last_day(y, m));
        while x.weekday() != wd {
            x = add(x, -1);
        }
        return x;
    }
    let mut x = ymd(y, m, 1);
    while x.weekday() != wd {
        x = add(x, 1);
    }
    add(x, 7 * (n as i64 - 1))
}

/// Ley Emiliani: the holiday moves to the next Monday unless it is one.
fn next_monday(x: NaiveDate) -> NaiveDate {
    let back = x.weekday().num_days_from_monday() as i64;
    if back == 0 { x } else { add(x, 7 - back) }
}

/// US rule: Saturday is observed on Friday, Sunday on Monday.
fn observed(x: NaiveDate) -> NaiveDate {
    match x.weekday() {
        Weekday::Sat => add(x, -1),
        Weekday::Sun => add(x, 1),
        _ => x,
    }
}

/// The holidays of `c` in year `y`, by date. US observed days can fall in the year before
/// (a Saturday Jan 1 is observed on Dec 31): they are listed under the holiday's own year.
pub fn holidays(c: Country, y: i32) -> Vec<Holiday> {
    let e = easter(y);
    let ed = |n: i64| add(e, n);
    let d = |m, day| ymd(y, m, day);
    let v: Vec<(NaiveDate, &'static str, &'static str)> = match c {
        Country::Co => vec![
            (d(1, 1), "co_new_year", "Año Nuevo"),
            (next_monday(d(1, 6)), "co_epiphany", "Día de los Reyes Magos"),
            (next_monday(d(3, 19)), "co_saint_joseph", "Día de San José"),
            (ed(-3), "co_holy_thursday", "Jueves Santo"),
            (ed(-2), "co_good_friday", "Viernes Santo"),
            (d(5, 1), "co_labor_day", "Día del Trabajo"),
            (next_monday(ed(39)), "co_ascension", "Ascensión del Señor"),
            (next_monday(ed(60)), "co_corpus_christi", "Corpus Christi"),
            (next_monday(ed(68)), "co_sacred_heart", "Sagrado Corazón"),
            (next_monday(d(6, 29)), "co_saints_peter_paul", "San Pedro y San Pablo"),
            (d(7, 20), "co_independence", "Día de la Independencia"),
            (d(8, 7), "co_boyaca", "Batalla de Boyacá"),
            (next_monday(d(8, 15)), "co_assumption", "La Asunción de la Virgen"),
            (next_monday(d(10, 12)), "co_race_day", "Día de la Raza"),
            (next_monday(d(11, 1)), "co_all_saints", "Todos los Santos"),
            (next_monday(d(11, 11)), "co_cartagena", "Independencia de Cartagena"),
            (d(12, 8), "co_immaculate", "Inmaculada Concepción"),
            (d(12, 25), "co_christmas", "Navidad"),
        ],
        Country::Us => {
            let mut v = vec![
                (observed(d(1, 1)), "us_new_year", "New Year's Day"),
                (nth(y, 1, Weekday::Mon, 3), "us_mlk", "Martin Luther King Jr. Day"),
                (nth(y, 2, Weekday::Mon, 3), "us_presidents", "Washington's Birthday"),
                (nth(y, 5, Weekday::Mon, -1), "us_memorial", "Memorial Day"),
                (observed(d(7, 4)), "us_independence", "Independence Day"),
                (nth(y, 9, Weekday::Mon, 1), "us_labor", "Labor Day"),
                (nth(y, 10, Weekday::Mon, 2), "us_columbus", "Columbus Day"),
                (observed(d(11, 11)), "us_veterans", "Veterans Day"),
                (nth(y, 11, Weekday::Thu, 4), "us_thanksgiving", "Thanksgiving Day"),
                (observed(d(12, 25)), "us_christmas", "Christmas Day"),
            ];
            if y >= 2021 {
                v.push((observed(d(6, 19)), "us_juneteenth", "Juneteenth"));
            }
            v
        }
        // Wyoming state offices: the federal days without Columbus Day and Juneteenth
        Country::UsWy => vec![
            (observed(d(1, 1)), "us_wy_new_year", "New Year's Day"),
            (nth(y, 1, Weekday::Mon, 3), "us_wy_equality_day", "Martin Luther King Jr./Wyoming Equality Day"),
            (nth(y, 2, Weekday::Mon, 3), "us_wy_presidents", "Presidents' Day"),
            (nth(y, 5, Weekday::Mon, -1), "us_wy_memorial", "Memorial Day"),
            (observed(d(7, 4)), "us_wy_independence", "Independence Day"),
            (nth(y, 9, Weekday::Mon, 1), "us_wy_labor", "Labor Day"),
            (observed(d(11, 11)), "us_wy_veterans", "Veterans Day"),
            (nth(y, 11, Weekday::Thu, 4), "us_wy_thanksgiving", "Thanksgiving Day"),
            (observed(d(12, 25)), "us_wy_christmas", "Christmas Day"),
        ],
        Country::Es => vec![
            (d(1, 1), "es_new_year", "Año Nuevo"),
            (d(1, 6), "es_epiphany", "Epifanía del Señor"),
            (ed(-2), "es_good_friday", "Viernes Santo"),
            (d(5, 1), "es_labor_day", "Fiesta del Trabajo"),
            (d(8, 15), "es_assumption", "Asunción de la Virgen"),
            (d(10, 12), "es_national_day", "Fiesta Nacional de España"),
            (d(11, 1), "es_all_saints", "Todos los Santos"),
            (d(12, 6), "es_constitution", "Día de la Constitución"),
            (d(12, 8), "es_immaculate", "Inmaculada Concepción"),
            (d(12, 25), "es_christmas", "Navidad"),
        ],
        Country::Mx => vec![
            (d(1, 1), "mx_new_year", "Año Nuevo"),
            (nth(y, 2, Weekday::Mon, 1), "mx_constitution", "Día de la Constitución"),
            (nth(y, 3, Weekday::Mon, 3), "mx_juarez", "Natalicio de Benito Juárez"),
            (d(5, 1), "mx_labor_day", "Día del Trabajo"),
            (d(9, 16), "mx_independence", "Día de la Independencia"),
            (nth(y, 11, Weekday::Mon, 3), "mx_revolution", "Día de la Revolución"),
            (d(12, 25), "mx_christmas", "Navidad"),
        ],
        Country::Br => {
            let mut v = vec![
                (d(1, 1), "br_new_year", "Confraternização Universal"),
                (ed(-2), "br_good_friday", "Sexta-feira Santa"),
                (d(4, 21), "br_tiradentes", "Tiradentes"),
                (d(5, 1), "br_labor_day", "Dia do Trabalho"),
                (d(9, 7), "br_independence", "Independência do Brasil"),
                (d(10, 12), "br_aparecida", "Nossa Senhora Aparecida"),
                (d(11, 2), "br_all_souls", "Finados"),
                (d(11, 15), "br_republic", "Proclamação da República"),
                (d(12, 25), "br_christmas", "Natal"),
            ];
            if y >= 2024 {
                v.push((d(11, 20), "br_black_consciousness", "Dia Nacional de Zumbi e da Consciência Negra"));
            }
            v
        }
        Country::Ca => {
            let mut v = vec![
                (d(1, 1), "ca_new_year", "New Year's Day"),
                (ed(-2), "ca_good_friday", "Good Friday"),
                // the Monday before May 25
                (next_monday(d(5, 18)), "ca_victoria", "Victoria Day"),
                (d(7, 1), "ca_canada_day", "Canada Day"),
                (nth(y, 9, Weekday::Mon, 1), "ca_labour", "Labour Day"),
                (nth(y, 10, Weekday::Mon, 2), "ca_thanksgiving", "Thanksgiving"),
                (d(11, 11), "ca_remembrance", "Remembrance Day"),
                (d(12, 25), "ca_christmas", "Christmas Day"),
                (d(12, 26), "ca_boxing_day", "Boxing Day"),
            ];
            if y >= 2021 {
                v.push((d(9, 30), "ca_truth_reconciliation", "National Day for Truth and Reconciliation"));
            }
            v
        }
    };
    let mut v: Vec<Holiday> = v.into_iter().map(|(date, id, name)| Holiday { date, id, name, country: c }).collect();
    v.sort_by_key(|h| h.date);
    v
}

/// The holidays of `countries` between `from` and `to` (inclusive), by date then country.
pub fn between(countries: &[Country], from: NaiveDate, to: NaiveDate) -> Vec<Holiday> {
    if to < from {
        return Vec::new();
    }
    let mut v: Vec<Holiday> = countries
        .iter()
        .flat_map(|c| (from.year()..=to.year() + 1).flat_map(move |y| holidays(*c, y)))
        .filter(|h| h.date >= from && h.date <= to)
        .collect();
    v.sort_by_key(|h| (h.date, h.country));
    v.dedup_by(|a, b| a.date == b.date && a.id == b.id);
    v
}

pub fn is_holiday(c: Country, x: NaiveDate) -> bool {
    [x.year(), x.year() + 1].iter().any(|y| holidays(c, *y).iter().any(|h| h.date == x))
}

/// Not a Saturday, Sunday or (when a country is given) a public holiday there.
pub fn is_business_day(x: NaiveDate, c: Option<Country>) -> bool {
    !matches!(x.weekday(), Weekday::Sat | Weekday::Sun) && !c.is_some_and(|c| is_holiday(c, x))
}

/// Which way a date moves when it falls on a non-business day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shift {
    /// The business day before (filings due "by" a date).
    Previous,
    /// The business day after (most legal deadlines roll forward).
    Next,
}

/// `x`, or the nearest business day in direction `shift`.
pub fn shift(x: NaiveDate, shift: Shift, c: Option<Country>) -> NaiveDate {
    let step = if shift == Shift::Previous { -1 } else { 1 };
    let mut d = x;
    // runs of non-business days are short; the bound only guards bad input
    for _ in 0..30 {
        if is_business_day(d, c) {
            return d;
        }
        d = add(d, step);
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dates(c: Country, y: i32) -> Vec<String> {
        let mut v: Vec<String> = holidays(c, y).iter().map(|h| h.date.format("%m-%d").to_string()).collect();
        v.dedup();
        v
    }

    #[test]
    fn easter_dates() {
        let e: Vec<String> = [2024, 2025, 2026, 2027, 2028, 2038].iter().map(|y| easter(*y).to_string()).collect();
        assert_eq!(e, ["2024-03-31", "2025-04-20", "2026-04-05", "2027-03-28", "2028-04-16", "2038-04-25"]);
    }

    #[test]
    fn colombia() {
        assert_eq!(dates(Country::Co, 2026), ["01-01", "01-12", "03-23", "04-02", "04-03", "05-01", "05-18", "06-08", "06-15", "06-29", "07-20", "08-07", "08-17", "10-12", "11-02", "11-16", "12-08", "12-25"]);
        assert_eq!(dates(Country::Co, 2025), ["01-01", "01-06", "03-24", "04-17", "04-18", "05-01", "06-02", "06-23", "06-30", "07-20", "08-07", "08-18", "10-13", "11-03", "11-17", "12-08", "12-25"]);
        assert_eq!(holidays(Country::Co, 2025).len(), 18);
        assert_eq!(dates(Country::Co, 2027), ["01-01", "01-11", "03-22", "03-25", "03-26", "05-01", "05-10", "05-31", "06-07", "07-05", "07-20", "08-07", "08-16", "10-18", "11-01", "11-15", "12-08", "12-25"]);
    }

    #[test]
    fn others() {
        assert_eq!(dates(Country::Us, 2026), ["01-01", "01-19", "02-16", "05-25", "06-19", "07-03", "09-07", "10-12", "11-11", "11-26", "12-25"]);
        assert!(is_holiday(Country::Us, ymd(2027, 12, 31)));
        assert!(!dates(Country::UsWy, 2026).contains(&"10-12".to_string()));
        assert_eq!(dates(Country::UsWy, 2026).len(), 9);
        assert_eq!(dates(Country::Mx, 2026), ["01-01", "02-02", "03-16", "05-01", "09-16", "11-16", "12-25"]);
        assert_eq!(dates(Country::Ca, 2026)[2], "05-18");
        assert_eq!(dates(Country::Ca, 2025)[2], "05-19");
        assert!(dates(Country::Br, 2026).contains(&"11-20".to_string()));
        assert!(dates(Country::Es, 2026).contains(&"04-03".to_string()));
        assert!(Country::ALL.iter().all(|c| Country::from_id(c.id()) == Some(*c)));
        assert_eq!(Country::from_id("US_WY"), Some(Country::UsWy));
        let span = between(&[Country::Co, Country::Us], ymd(2026, 12, 1), ymd(2027, 1, 31));
        let got: Vec<(String, &str)> = span.iter().take(3).map(|h| (h.date.to_string(), h.id)).collect();
        assert_eq!(got, [("2026-12-08".to_string(), "co_immaculate"), ("2026-12-25".to_string(), "co_christmas"), ("2026-12-25".to_string(), "us_christmas")]);
        // every id is unique within a country and year, and prefixed by the country
        for c in Country::ALL {
            let h = holidays(c, 2026);
            let mut ids: Vec<&str> = h.iter().map(|h| h.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), h.len());
            assert!(h.iter().all(|h| h.id.starts_with(&c.id().replace('-', "_"))));
        }
    }

    #[test]
    fn business_days() {
        let co = Some(Country::Co);
        assert_eq!(shift(ymd(2026, 8, 15), Shift::Next, co), ymd(2026, 8, 18));
        assert_eq!(shift(ymd(2026, 8, 15), Shift::Previous, co), ymd(2026, 8, 14));
        assert_eq!(shift(ymd(2026, 4, 3), Shift::Previous, co), ymd(2026, 4, 1));
        assert_eq!(shift(ymd(2026, 8, 15), Shift::Next, None), ymd(2026, 8, 17));
        assert!(is_business_day(ymd(2026, 11, 20), co));
        assert!(!is_business_day(ymd(2026, 11, 16), co));
        assert!(is_business_day(ymd(2026, 11, 16), Some(Country::Us)));
    }
}
