//! Canonical JSON, [`FORMAT`], and the ids made from it: one text for one
//! value, so that a record's SHA-256 names what it says and not how it was
//! written.
//!
//! The rules (mvp-environments' identity rules, versioned here):
//!
//! - an object's members are sorted by key, compared as UTF-8 bytes (which
//!   is the order of their code points); the order a map keeps, or a
//!   feature of serde_json that keeps insertion order, does not matter;
//! - a member whose value is `null` is left out, so that an unknown value
//!   and an absent one give the same text; a `null` in an array stays,
//!   since its place counts;
//! - an array keeps its order;
//! - no white space;
//! - a string is escaped as serde_json escapes it: `"` and `\`, control
//!   characters as `\n`, `\t` or `\u00XX`, everything else as UTF-8;
//! - an integer (one that JSON wrote without `.` or exponent, up to the
//!   range of `i64` and `u64`) is written in decimal, `-` for a negative;
//! - a float is written with the fewest significant digits that read back
//!   to the same `f64` (serde_json's form). With the value `m × 10^k`,
//!   `1 ≤ m < 10`: for `-5 ≤ k ≤ 15` positionally, always with a `.` and
//!   a digit after it (`100.0`, `0.00001`); otherwise as the digits of `m`,
//!   with a `.` only when there are several, then `e`, the sign, `+` or
//!   `-`, and `k` without leading zeros (`1e+16`, `9.5e-6`). `-0.0` is
//!   written `0.0`. A float is never the integer of the same value: `1.0`
//!   is not `1`. JSON has no NaN and no infinity, so a value that holds one
//!   cannot be made; a reader that meets one refuses the file.
//!
//! ```
//! use helpers::canonical;
//! use serde_json::json;
//!
//! let record = json!({"name": "shadow", "addons": [], "format": "kai-variant/1", "note": null});
//! assert_eq!(
//!     canonical::to_string(&record),
//!     r#"{"addons":[],"format":"kai-variant/1","name":"shadow"}"#
//! );
//! assert!(canonical::id(&record).starts_with("sha256:"));
//! ```

use serde_json::Value;

use crate::hash::sha256_bytes;

/// The version of these rules. A change that gives any value another text
/// is a new number, and every id made with the old one stays what it was.
pub const FORMAT: &str = "kai-canonical-json/1";

/// The canonical text of `value`.
pub fn to_string(value: &Value) -> String {
    let mut out = String::new();
    write(value, &mut out);
    out
}

/// The id of `value`: `sha256:` and the SHA-256 of its canonical text, in
/// lower-case hex.
pub fn id(value: &Value) -> String {
    format!("sha256:{}", sha256_bytes(to_string(value).as_bytes()))
}

fn write(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        // -0.0 equals 0.0 and has its text.
        Value::Number(number) if number.as_f64() == Some(0.0) && number.is_f64() => {
            out.push_str("0.0");
        }
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => string(text, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        Value::Object(members) => {
            let mut members: Vec<(&String, &Value)> = members
                .iter()
                .filter(|(_, value)| !value.is_null())
                .collect();
            members.sort_unstable_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            out.push('{');
            for (i, (key, value)) in members.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                string(key, out);
                out.push(':');
                write(value, out);
            }
            out.push('}');
        }
    }
}

fn string(text: &str, out: &mut String) {
    // A string always serializes; serde_json's escaping is the rule's.
    out.push_str(&serde_json::to_string(text).expect("a string serializes"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn members_are_sorted_by_their_bytes_and_nested_ones_too() {
        let value = json!({"b": 1, "a": {"z": [3, 1, 2], "é": 0, "Z": 0}, "B": 2});
        assert_eq!(
            to_string(&value),
            r#"{"B":2,"a":{"Z":0,"z":[3,1,2],"é":0},"b":1}"#
        );
    }

    #[test]
    fn the_order_a_map_keeps_does_not_matter() {
        let mut first = serde_json::Map::new();
        first.insert("b".into(), json!(1));
        first.insert("a".into(), json!(2));
        let mut second = serde_json::Map::new();
        second.insert("a".into(), json!(2));
        second.insert("b".into(), json!(1));
        assert_eq!(id(&Value::Object(first)), id(&Value::Object(second)));
    }

    #[test]
    fn a_null_member_is_absent_but_a_null_in_an_array_is_kept() {
        assert_eq!(id(&json!({"a": 1, "unknown": null})), id(&json!({"a": 1})));
        assert_eq!(to_string(&json!([null, 1])), "[null,1]");
        assert_eq!(to_string(&json!(null)), "null");
    }

    #[test]
    fn the_documented_example_holds() {
        let example: Value = serde_json::from_str(include_str!(
            "../../../contracts/kai-canonical-json-1.example.json"
        ))
        .unwrap();
        assert_eq!(to_string(&example["value"]), example["text"]);
        assert_eq!(id(&example["value"]), example["id"]);
    }

    #[test]
    fn control_characters_have_short_escapes_or_lower_case_hex() {
        assert_eq!(
            to_string(&json!("\u{8}\u{c}\t\r\u{1f}\u{7f}/")),
            "\"\\b\\f\\t\\r\\u001f\u{7f}/\""
        );
    }

    #[test]
    fn strings_are_escaped_as_serde_json_escapes_them() {
        assert_eq!(
            to_string(&json!("a\"b\\c\n\u{1}ü")),
            r#""a\"b\\c\n\u0001ü""#
        );
    }

    #[test]
    fn integers_and_floats_have_one_text_each() {
        assert_eq!(
            to_string(&json!([0, -1, u64::MAX, i64::MIN])),
            "[0,-1,18446744073709551615,-9223372036854775808]"
        );
        assert_eq!(
            to_string(&json!([0.2, 0.15, 1.0, -0.5])),
            "[0.2,0.15,1.0,-0.5]"
        );
        assert_ne!(id(&json!(1)), id(&json!(1.0)));
        assert_eq!(to_string(&json!(-0.0)), "0.0");
        assert_eq!(to_string(&json!(0.1 + 0.2)), "0.30000000000000004");
    }

    #[test]
    fn a_float_is_positional_from_1e_minus_5_to_below_1e16_and_else_has_an_exponent() {
        let texts: Vec<String> = [
            1e-5,
            9.5e-6,
            1.5e15,
            9.999e15,
            1e16,
            1.5e16,
            1e21,
            5e-324,
            f64::MAX,
        ]
        .iter()
        .map(|f| to_string(&json!(f)))
        .collect();
        assert_eq!(
            texts,
            [
                "0.00001",
                "9.5e-6",
                "1500000000000000.0",
                "9999000000000000.0",
                "1e+16",
                "1.5e+16",
                "1e+21",
                "5e-324",
                "1.7976931348623157e+308",
            ]
        );
    }

    #[test]
    fn the_id_is_the_sha256_of_the_text() {
        let value = json!({"b": [true, false], "a": "x"});
        assert_eq!(to_string(&value), r#"{"a":"x","b":[true,false]}"#);
        assert_eq!(
            id(&value),
            format!("sha256:{}", sha256_bytes(br#"{"a":"x","b":[true,false]}"#))
        );
        assert_eq!(
            id(&json!({})),
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
    }
}
