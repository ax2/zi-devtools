use serde::{Serialize, Serializer};
use serde_json::value::RawValue;
use std::borrow::Cow;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Scale {
    Small(i128),
    Large { negative: bool, digits: Box<str> },
}

fn scale(exponent: &str, adjustment: i64) -> Scale {
    let negative = exponent.starts_with('-');
    let digits = exponent
        .trim_start_matches(['-', '+'])
        .trim_start_matches('0');
    if digits.len() <= 38 {
        let magnitude = digits.parse::<i128>().unwrap_or(0);
        let value = if negative { -magnitude } else { magnitude };
        return Scale::Small(value + i128::from(adjustment));
    }

    // An exponent with thirty-nine or more significant digits cannot change sign
    // when adjusted by the fraction length within a bounded document.
    let mut shifted = digits.as_bytes().to_vec();
    let mut carry = if negative { -adjustment } else { adjustment };
    for digit in shifted.iter_mut().rev() {
        if carry == 0 {
            break;
        }
        let value = i64::from(*digit - b'0') + carry;
        *digit = b'0' + value.rem_euclid(10) as u8;
        carry = value.div_euclid(10);
    }
    debug_assert!(carry >= 0);
    let digits = String::from_utf8(shifted).expect("ASCII exponent");
    let digits = if carry == 0 {
        digits.trim_start_matches('0').to_owned()
    } else {
        format!("{carry}{digits}")
    };
    let signed = if negative {
        format!("-{digits}")
    } else {
        digits.clone()
    };
    if let Ok(value) = signed.parse::<i128>() {
        Scale::Small(value)
    } else {
        Scale::Large {
            negative,
            digits: digits.into_boxed_str(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key<'a> {
    decimal: bool,
    negative: bool,
    significant: Cow<'a, str>,
    scale: Scale,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Number<'a>(pub(super) &'a str);

impl<'a> Number<'a> {
    fn key(self) -> Key<'a> {
        let raw = self.0;
        // Existing comparison distinguishes the integer and decimal kinds.
        // A bare negative zero was parsed as a decimal by serde_json.
        let decimal = raw.contains(['.', 'e', 'E']) || raw == "-0";
        let negative = raw.starts_with('-');
        let unsigned = raw.strip_prefix('-').unwrap_or(raw);
        let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let digits = if fraction.is_empty() {
            Cow::Borrowed(whole)
        } else {
            Cow::Owned(format!("{whole}{fraction}"))
        };
        let trimmed = digits.trim_start_matches('0');
        if trimmed.is_empty() {
            Key {
                decimal,
                negative: false,
                significant: Cow::Borrowed("0"),
                scale: Scale::Small(0),
            }
        } else {
            let significant_len = trimmed.trim_end_matches('0').len();
            let adjustment = (trimmed.len() - significant_len) as i64 - fraction.len() as i64;
            let significant = match digits {
                Cow::Borrowed(value) => {
                    Cow::Borrowed(&value.trim_start_matches('0')[..significant_len])
                }
                Cow::Owned(value) => {
                    Cow::Owned(value.trim_start_matches('0')[..significant_len].to_owned())
                }
            };
            Key {
                decimal,
                negative,
                significant,
                scale: scale(exponent, adjustment),
            }
        }
    }
}

impl PartialEq for Number<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0 || self.key() == other.key()
    }
}

impl Eq for Number<'_> {}

impl PartialOrd for Number<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Number<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if self.0 == other.0 {
            std::cmp::Ordering::Equal
        } else {
            self.key().cmp(&other.key())
        }
    }
}

impl Serialize for Number<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let raw: &RawValue = serde_json::from_str(self.0).map_err(serde::ser::Error::custom)?;
        raw.serialize(serializer)
    }
}
