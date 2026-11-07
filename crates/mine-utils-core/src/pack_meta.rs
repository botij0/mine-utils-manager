use serde_json::Value;

use crate::pack::{FormatRange, FormatVersion, PackError, PackMeta};

pub const MAX_DESCRIPTION_CHARS: usize = 300;

/// Parse the bytes of pack.mcmeta into the normalized meta. Unknown extra keys
/// are ignored so a newer pack never fails the scan.
pub fn parse_pack_meta(bytes: &[u8]) -> Result<PackMeta, PackError> {
    // Some published packs append a license line after the JSON object.
    // Minecraft reads the first value, so do the same and ignore trailing text.
    let mut stream = serde_json::Deserializer::from_slice(bytes).into_iter::<Value>();
    let root = stream
        .next()
        .transpose()
        .map_err(|err| PackError::BrokenArchive {
            name: "pack.mcmeta".to_string(),
            reason: format!("invalid JSON: {err}"),
        })?
        .ok_or_else(|| PackError::BrokenArchive {
            name: "pack.mcmeta".to_string(),
            reason: "empty pack.mcmeta".to_string(),
        })?;
    let pack = root.get("pack").ok_or_else(|| PackError::BrokenArchive {
        name: "pack.mcmeta".to_string(),
        reason: "missing pack object".to_string(),
    })?;

    let pack_format = pack
        .get("pack_format")
        .and_then(|value| parse_format(value, Bound::Exact));
    let min_format = pack
        .get("min_format")
        .and_then(|value| parse_format(value, Bound::Exact));
    let max_format = pack
        .get("max_format")
        .and_then(|value| parse_format(value, Bound::Upper));
    let supported_formats = pack.get("supported_formats").and_then(parse_supported);

    let description = pack
        .get("description")
        .map(flatten_component)
        .map(|text| strip_section_codes(&text))
        .map(|text| text.trim().to_string())
        .unwrap_or_default();

    Ok(PackMeta {
        pack_format,
        min_format,
        max_format,
        supported_formats,
        description: cap_description(&description),
    })
}

/// How a bare major number reads. A lower bound or a pack's own format is that
/// exact version (`82` is `82.0`); an upper bound written without a minor
/// accepts every minor of that major.
#[derive(Clone, Copy)]
enum Bound {
    Exact,
    Upper,
}

impl Bound {
    fn whole_major(self, major: u32) -> FormatVersion {
        match self {
            Bound::Exact => FormatVersion::from_major(major),
            Bound::Upper => FormatVersion::any_minor_of(major),
        }
    }
}

fn parse_format(value: &Value, bound: Bound) -> Option<FormatVersion> {
    match value {
        Value::Number(number) => {
            if let Some(major) = number.as_u64() {
                return Some(bound.whole_major(u32::try_from(major).ok()?));
            }
            let float = number.as_f64()?;
            let major = float.trunc();
            if !(0.0..=u32::MAX as f64).contains(&major) {
                return None;
            }
            let minor = ((float - major) * 10.0).round();
            Some(FormatVersion::new(major as u32, minor as u32))
        }
        Value::Array(items) => match items.as_slice() {
            [major] => Some(bound.whole_major(u32::try_from(major.as_u64()?).ok()?)),
            [major, minor] => Some(FormatVersion::new(
                u32::try_from(major.as_u64()?).ok()?,
                u32::try_from(minor.as_u64()?).ok()?,
            )),
            _ => None,
        },
        Value::String(text) => parse_format_string(text, bound),
        _ => None,
    }
}

fn parse_format_string(text: &str, bound: Bound) -> Option<FormatVersion> {
    let text = text.trim();
    match text.split_once('.') {
        Some((major, minor)) => Some(FormatVersion::new(major.parse().ok()?, minor.parse().ok()?)),
        None => Some(bound.whole_major(text.parse().ok()?)),
    }
}

/// `supported_formats` lists major versions, so its upper end covers every
/// minor of that major. An integer or one-element array is a single major.
fn parse_supported(value: &Value) -> Option<FormatRange> {
    match value {
        Value::Array(items) => match items.as_slice() {
            [major] => single_major(major),
            [min, max] => Some(FormatRange {
                min: Some(parse_format(min, Bound::Exact)?),
                max: Some(parse_format(max, Bound::Upper)?),
            }),
            _ => None,
        },
        Value::Number(_) => single_major(value),
        Value::Object(map) => Some(FormatRange {
            min: Some(
                map.get("min_inclusive")
                    .and_then(|value| parse_format(value, Bound::Exact))?,
            ),
            max: Some(
                map.get("max_inclusive")
                    .and_then(|value| parse_format(value, Bound::Upper))?,
            ),
        }),
        _ => None,
    }
}

fn single_major(value: &Value) -> Option<FormatRange> {
    Some(FormatRange {
        min: Some(parse_format(value, Bound::Exact)?),
        max: Some(parse_format(value, Bound::Upper)?),
    })
}

/// Flatten a Minecraft text component. Strings, `{"text": ...}` objects, and
/// arrays all become one line, with `extra` children appended in order.
fn flatten_component(value: &Value) -> String {
    let mut out = String::new();
    append_component(value, &mut out);
    out
}

fn append_component(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => out.push_str(text),
        Value::Array(items) => {
            for item in items {
                append_component(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                out.push_str(text);
            } else if let Some(key) = map.get("translate").and_then(Value::as_str) {
                out.push_str(key);
            }
            if let Some(extra) = map.get("extra") {
                append_component(extra, out);
            }
        }
        _ => {}
    }
}

/// Drop `§x` colour and style codes, including the trailing code character.
fn strip_section_codes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '§' {
            chars.next();
        } else {
            out.push(ch);
        }
    }
    out
}

fn cap_description(text: &str) -> String {
    let mut out: String = text.chars().take(MAX_DESCRIPTION_CHARS).collect();
    if text.chars().count() > MAX_DESCRIPTION_CHARS {
        out.pop();
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> PackMeta {
        parse_pack_meta(json.as_bytes()).unwrap()
    }

    #[test]
    fn string_description() {
        let meta = parse(r#"{"pack":{"pack_format":55,"description":"New book covers"}}"#);
        assert_eq!(meta.description, "New book covers");
        assert_eq!(meta.pack_format, Some(FormatVersion::from_major(55)));
        assert_eq!(meta.effective_range().unwrap().label(), "55");
    }

    #[test]
    fn object_and_array_descriptions() {
        let meta = parse(
            r#"{"pack":{"pack_format":15,"description":{"text":"Both ","extra":[{"text":"kinds"},{"text":" here"}]}}}"#,
        );
        assert_eq!(meta.description, "Both kinds here");

        let meta =
            parse(r#"{"pack":{"pack_format":15,"description":["one ","two ",{"text":"three"}]}}"#);
        assert_eq!(meta.description, "one two three");
    }

    #[test]
    fn translate_description() {
        let meta = parse(r#"{"pack":{"description":{"translate":"pack.name"}}}"#);
        assert_eq!(meta.description, "pack.name");
    }

    #[test]
    fn strips_section_codes_and_caps_length() {
        let long = "x".repeat(400);
        let meta = parse(&format!(
            r#"{{"pack":{{"pack_format":7,"description":"§aGreen §r{line}"}}}}"#,
            line = long
        ));
        assert!(meta.description.starts_with("Green x"));
        assert!(!meta.description.contains('§'));
        assert_eq!(meta.description.chars().count(), MAX_DESCRIPTION_CHARS);
        assert!(meta.description.ends_with('…'));
    }

    #[test]
    fn dotted_and_array_formats() {
        let meta = parse(
            r#"{"pack":{"pack_format":34,"min_format":[69,0],"max_format":"75.0","supported_formats":[34,69]}}"#,
        );
        assert_eq!(meta.min_format, Some(FormatVersion::new(69, 0)));
        assert_eq!(meta.max_format, Some(FormatVersion::new(75, 0)));
        assert_eq!(meta.effective_range().unwrap().label(), "69 to 75");
    }

    #[test]
    fn one_element_and_integer_formats_follow_the_mcmeta_spec() {
        let meta = parse(r#"{"pack":{"min_format":[82],"max_format":94}}"#);
        assert_eq!(meta.min_format, Some(FormatVersion::new(82, 0)));
        assert_eq!(meta.max_format, Some(FormatVersion::new(94, 0x7fff_ffff)));

        let meta = parse(r#"{"pack":{"min_format":82,"max_format":[94]}}"#);
        assert_eq!(meta.min_format, Some(FormatVersion::new(82, 0)));
        assert_eq!(meta.max_format, Some(FormatVersion::new(94, 0x7fff_ffff)));

        let meta = parse(r#"{"pack":{"min_format":[82,1],"max_format":[94,1]}}"#);
        assert_eq!(meta.min_format, Some(FormatVersion::new(82, 1)));
        assert_eq!(meta.max_format, Some(FormatVersion::new(94, 1)));
    }

    #[test]
    fn supported_formats_accepts_an_integer_and_a_single_element_array() {
        for json in [
            r#"{"pack":{"supported_formats":42}}"#,
            r#"{"pack":{"supported_formats":[42]}}"#,
        ] {
            let range = parse(json).supported_formats.unwrap();
            assert_eq!(range.min, Some(FormatVersion::new(42, 0)), "{json}");
            assert_eq!(
                range.max,
                Some(FormatVersion::new(42, 0x7fff_ffff)),
                "{json}"
            );
            assert_eq!(range.label(), "42", "{json}");
        }
    }

    #[test]
    fn plain_string_formats_parse_as_a_major_number() {
        let meta = parse(r#"{"pack":{"pack_format":"34","description":"S"}}"#);
        assert_eq!(meta.pack_format, Some(FormatVersion::from_major(34)));
        assert_eq!(meta.effective_range().unwrap().label(), "34");

        let meta = parse(r#"{"pack":{"min_format":"48","description":"S"}}"#);
        assert_eq!(meta.min_format, Some(FormatVersion::from_major(48)));
        assert_eq!(meta.effective_range().unwrap().label(), "48 and up");
    }

    #[test]
    fn supported_formats_object_is_inclusive() {
        let meta = parse(
            r#"{"pack":{"pack_format":34,"supported_formats":{"min_inclusive":34,"max_inclusive":69}}}"#,
        );
        let range = meta.supported_formats.unwrap();
        assert_eq!(range.min, Some(FormatVersion::from_major(34)));
        assert_eq!(range.max, Some(FormatVersion::new(69, 0x7fff_ffff)));
    }

    #[test]
    fn trailing_text_after_json_is_ignored() {
        let meta = parse_pack_meta(
            br#"{"pack":{"pack_format":55,"description":"Covers"}}
Re-covered (c) 2025 by carl_classic"#,
        )
        .unwrap();
        assert_eq!(meta.description, "Covers");
        assert_eq!(meta.pack_format, Some(FormatVersion::from_major(55)));
    }

    #[test]
    fn missing_pack_object_is_an_error() {
        let err = parse_pack_meta(br#"{"overlays":{}}"#).unwrap_err();
        assert_eq!(err.code(), "pack_broken_archive");
        assert!(err.to_string().contains("missing pack object"), "{err}");
    }
}
