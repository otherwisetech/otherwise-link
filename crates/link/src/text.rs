//! Text folding for search and duplicate detection: lower case, Latin accents removed,
//! punctuation collapsed to single spaces. "José  Pérez-Gómez" → "jose perez gomez".

/// Accent-free lower-case form of `c` (Latin-1 + Latin Extended-A, which covers es/pt/fr/de).
fn fold_char(c: char, out: &mut String) {
    let base = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' => "a",
        'æ' | 'Æ' => "ae",
        'ç' | 'ć' | 'č' | 'Ç' | 'Ć' | 'Č' => "c",
        'ď' | 'đ' | 'Ď' | 'Đ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' | 'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ė' | 'Ę' | 'Ě' => "e",
        'ğ' | 'Ğ' => "g",
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' | 'Ì' | 'Í' | 'Î' | 'Ï' | 'Ī' | 'Į' | 'İ' => "i",
        'ł' | 'ľ' | 'Ł' | 'Ľ' => "l",
        'ñ' | 'ń' | 'ň' | 'Ñ' | 'Ń' | 'Ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ō' | 'Ő' => "o",
        'œ' | 'Œ' => "oe",
        'ř' | 'Ř' => "r",
        'ś' | 'š' | 'ş' | 'Ś' | 'Š' | 'Ş' => "s",
        'ß' => "ss",
        'ť' | 'ţ' | 'Ť' | 'Ţ' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'ų' | 'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ū' | 'Ů' | 'Ű' | 'Ų' => "u",
        'ý' | 'ÿ' | 'Ý' | 'Ÿ' => "y",
        'ź' | 'ż' | 'ž' | 'Ź' | 'Ż' | 'Ž' => "z",
        // combining marks (text in NFD, as macOS file names are)
        '\u{0300}'..='\u{036f}' => "",
        _ => {
            out.extend(c.to_lowercase());
            return;
        }
    };
    out.push_str(base);
}

pub fn fold(s: &str) -> String {
    let mut raw = String::with_capacity(s.len());
    for c in s.chars() {
        fold_char(c, &mut raw);
    }
    let mut out = String::with_capacity(raw.len());
    for word in raw.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Only letters and digits, upper case: tax ids, plates, VINs ("ABC-123" → "ABC123").
pub fn compact_upper(s: &str) -> String {
    fold(s).chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_uppercase).collect()
}

/// At most `max` characters (not bytes), with `…` when cut.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding() {
        assert_eq!(fold("José  Pérez-Gómez"), "jose perez gomez");
        assert_eq!(fold("Vigi\u{0301}a"), "vigia");
        assert_eq!(fold("Straße, S.A.S."), "strasse s a s");
        assert_eq!(fold("  "), "");
        assert_eq!(compact_upper("abc-12 3"), "ABC123");
        assert_eq!(compact_upper("900.123.456-7"), "9001234567");
    }
}
