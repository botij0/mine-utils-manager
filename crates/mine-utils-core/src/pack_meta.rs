use serde_json::Value;

use crate::pack::{FormatRange, FormatVersion, PackError, PackMeta};

pub const MAX_DESCRIPTION_CHARS: usize = 300;

/// Parse the bytes of pack.mcmeta into the normalized meta. Unknown extra keys
/// are ignored so a newer pack never fails the scan.
pub fn parse_pack_meta(bytes: &[u8]) -> Result<PackMeta, PackError> {
    let root: Value = serde_json::from_slice(bytes).map_err(|err| PackError::BrokenArchive {
        name: "pack.mcmeta".to_string(),
        reason: format!("invalid JSON: {err}"),
    })?;
    let pack = root.get("pack").ok_or_else(|| PackError::BrokenArchive {
        name: "pack.mcmeta".to_string(),
        reason: "missing pack object".to_string(),
    })?;

    let pack_format = pack.get("pack_format").and_then(parse_format);
    let min_format = pack.get("min_format").and_then(parse_format);
    let max_format = pack.get("max_format").and_then(parse_format);
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

fn parse_format(value: &Value) -> Option<FormatVersion> {
    match value {
        Value::Number(number) => {
            if let Some(major) = number.as_u64() {
                return Some(FormatVersion::from_major(u32::try_from(major).ok()?));
            }
            let float = number.as_f64()?;
            let major = float.trunc();
            if !(0.0..=u32::MAX as f64).contains(&major) {
                return None;
            }
            let minor = ((float - major) * 10.0).round();
            Some(FormatVersion::new(major as u32, minor as u32))
        }
        Value::Array(items) if items.len() == 2 => Some(FormatVersion::new(
            u32::try_from(items[0].as_u64()?).ok()?,
            u32::try_from(items[1].as_u64()?).ok()?,
        )),
        Value::String(text) => parse_format_string(text),
        _ => None,
    }
}

fn parse_format_string(text: &str) -> Option<FormatVersion> {
    let text = text.trim();
    let (major, minor) = text.split_once('.')?;
    Some(FormatVersion::new(
        major.parse().ok()?,
        minor.parse().ok()?,
    ))
}

fn parse_supported(value: &Value) -> Option<FormatRange> {
    match value {
        Value::Array(items) if items.len() == 2 => Some(FormatRange {
            min: parse_format(&items[0])?,
            max: parse_format(&items[1])?,
        }),
        Value::Object(map) => Some(FormatRange {
            min: map.get("min_inclusive").and_then(parse_format)?,
            max: map.get("max_inclusive").and_then(parse_format)?,
        }),
        _ => None,
    }
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

        let meta = parse(
            r#"{"pack":{"pack_format":15,"description":["one ","two ",{"text":"three"}]}}"#,
        );
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
    fn supported_formats_object_is_inclusive() {
        let meta = parse(
            r#"{"pack":{"pack_format":34,"supported_formats":{"min_inclusive":34,"max_inclusive":69}}}"#,
        );
        let range = meta.supported_formats.unwrap();
        assert_eq!(range.min, FormatVersion::from_major(34));
        assert_eq!(range.max, FormatVersion::from_major(69));
    }

    #[test]
    fn missing_pack_object_is_an_error() {
        let err = parse_pack_meta(br#"{"overlays":{}}"#).unwrap_err();
        assert_eq!(err.code(), "pack_broken_archive");
        assert!(err.to_string().contains("missing pack object"), "{err}");
    }
}
