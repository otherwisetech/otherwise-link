//! Tax and registration ids: normalization and, where the country publishes one, the check-digit
//! rule. Unknown schemes are kept as given (normalized) and reported as [`Check::Unknown`].
use crate::text::compact_upper;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaxId {
    /// ISO 3166-1 alpha-2, upper case (`CO`, `US`, `BR`, `FR`, `MX`…).
    pub country: String,
    /// `NIT`, `CC`, `EIN`, `SSN`, `CNPJ`, `CPF`, `SIREN`, `SIRET`, `RFC`, `VAT`…
    pub scheme: String,
    /// Normalized: letters and digits only, upper case, check digit included (`9001234567`).
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Valid,
    Invalid,
    /// No rule for this scheme, or the value has no check digit to verify.
    Unknown,
}

impl TaxId {
    pub fn new(country: &str, scheme: &str, value: &str) -> TaxId {
        TaxId { country: country.trim().to_ascii_uppercase(), scheme: compact_upper(scheme), value: compact_upper(value) }
    }

    pub fn normalized(&self) -> TaxId {
        TaxId::new(&self.country, &self.scheme, &self.value)
    }

    /// The registry key: `tax:CO:NIT:9001234567`.
    pub fn key(&self) -> String {
        let n = self.normalized();
        format!("tax:{}:{}:{}", n.country, n.scheme, n.value)
    }

    pub fn check(&self) -> Check {
        let n = self.normalized();
        let digits = n.value.bytes().all(|b| b.is_ascii_digit());
        match (n.country.as_str(), n.scheme.as_str()) {
            ("CO", "NIT") if digits => match n.value.len() {
                10 => ok(nit_dv(&n.value[..9]) == n.value.as_bytes()[9] - b'0'),
                9 => Check::Unknown,
                _ => Check::Invalid,
            },
            ("US", "EIN") => ok(digits && n.value.len() == 9),
            ("US", "SSN") => ok(digits && n.value.len() == 9 && !n.value.starts_with("000") && !n.value.starts_with("666") && !n.value.starts_with('9')),
            ("BR", "CNPJ") => ok(digits && cnpj_ok(&n.value)),
            ("BR", "CPF") => ok(digits && cpf_ok(&n.value)),
            ("FR", "SIREN") => ok(digits && n.value.len() == 9 && luhn(&n.value)),
            ("FR", "SIRET") => ok(digits && n.value.len() == 14 && luhn(&n.value)),
            ("MX", "RFC") => ok(matches!(n.value.len(), 12 | 13) && n.value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'&')),
            _ => Check::Unknown,
        }
    }
}

fn ok(b: bool) -> Check {
    if b { Check::Valid } else { Check::Invalid }
}

/// DIAN's verification digit for a Colombian NIT (weights 3, 7, 13… mod 11).
pub fn nit_dv(nit: &str) -> u8 {
    const W: [u32; 15] = [3, 7, 13, 17, 19, 23, 29, 37, 41, 43, 47, 53, 59, 67, 71];
    let sum: u32 = nit.bytes().rev().zip(W).map(|(d, w)| (d - b'0') as u32 * w).sum();
    let r = sum % 11;
    if r > 1 { (11 - r) as u8 } else { r as u8 }
}

fn digits(s: &str) -> Vec<u32> {
    s.bytes().map(|b| (b - b'0') as u32).collect()
}

fn mod11_digit(ds: &[u32], weights: &[u32]) -> u32 {
    let r = ds.iter().zip(weights).map(|(d, w)| d * w).sum::<u32>() % 11;
    if r < 2 { 0 } else { 11 - r }
}

fn cnpj_ok(s: &str) -> bool {
    if s.len() != 14 || s.bytes().all(|b| b == s.as_bytes()[0]) {
        return false;
    }
    let d = digits(s);
    let w1 = [5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2];
    let w2 = [6, 5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2];
    mod11_digit(&d[..12], &w1) == d[12] && mod11_digit(&d[..13], &w2) == d[13]
}

fn cpf_ok(s: &str) -> bool {
    if s.len() != 11 || s.bytes().all(|b| b == s.as_bytes()[0]) {
        return false;
    }
    let d = digits(s);
    let w1: Vec<u32> = (2..=10).rev().collect();
    let w2: Vec<u32> = (2..=11).rev().collect();
    mod11_digit(&d[..9], &w1) == d[9] && mod11_digit(&d[..10], &w2) == d[10]
}

fn luhn(s: &str) -> bool {
    let sum: u32 = s
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, b)| {
            let d = (b - b'0') as u32;
            if i % 2 == 1 { if d * 2 > 9 { d * 2 - 9 } else { d * 2 } } else { d }
        })
        .sum();
    sum % 10 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks() {
        // DIAN's own NIT is 800197268-4
        assert_eq!(nit_dv("800197268"), 4);
        assert_eq!(TaxId::new("co", "nit", "800.197.268-4").check(), Check::Valid);
        assert_eq!(TaxId::new("CO", "NIT", "800197268-5").check(), Check::Invalid);
        assert_eq!(TaxId::new("CO", "NIT", "800197268").check(), Check::Unknown);
        assert_eq!(TaxId::new("US", "EIN", "12-3456789").check(), Check::Valid);
        assert_eq!(TaxId::new("US", "EIN", "12-345678").check(), Check::Invalid);
        assert_eq!(TaxId::new("BR", "CNPJ", "11.222.333/0001-81").check(), Check::Valid);
        assert_eq!(TaxId::new("BR", "CNPJ", "11.222.333/0001-82").check(), Check::Invalid);
        assert_eq!(TaxId::new("BR", "CPF", "529.982.247-25").check(), Check::Valid);
        assert_eq!(TaxId::new("BR", "CPF", "111.111.111-11").check(), Check::Invalid);
        assert_eq!(TaxId::new("FR", "SIREN", "732 829 320").check(), Check::Valid);
        assert_eq!(TaxId::new("FR", "SIREN", "732 829 321").check(), Check::Invalid);
        assert_eq!(TaxId::new("DE", "VAT", "DE123").check(), Check::Unknown);
        assert_eq!(TaxId::new("co", "nit", "800.197.268-4").key(), "tax:CO:NIT:8001972684");
    }
}
