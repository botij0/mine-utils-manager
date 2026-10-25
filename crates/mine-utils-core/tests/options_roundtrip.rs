//! Round-trip tests for `options.txt` parsing and writing.
//!
//! The committed fixtures here are synthetic. The operator's real file is
//! copied to `/tmp/opencode/pr-03-owner/fixtures/options-real.txt` and used
//! only when present, so the real bytes never land in the repository.

use std::path::{Path, PathBuf};

use mine_utils_core::options_txt::{read_options, write_options, OptionsTxt};
use mine_utils_core::resource_stack;

const REAL_COPY: &str = "/tmp/opencode/pr-03-owner/fixtures/options-real.txt";

fn real_copy() -> Option<PathBuf> {
    let path = PathBuf::from(REAL_COPY);
    path.is_file().then_some(path)
}

fn temp_options(text: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("options.txt");
    std::fs::write(&path, text).expect("write fixture");
    (dir, path)
}

fn line_with_key(text: &str, key: &str) -> String {
    text.lines()
        .find(|line| line.starts_with(&format!("{key}:")))
        .unwrap_or_else(|| panic!("no {key} line"))
        .to_string()
}

fn without_key(text: &str, key: &str) -> Vec<String> {
    let prefix = format!("{key}:");
    text.lines()
        .filter(|line| !line.starts_with(&prefix))
        .map(str::to_string)
        .collect()
}

fn assert_only_target_line_changed(before: &str, after: &str, key: &str) {
    assert_eq!(
        without_key(before, key),
        without_key(after, key),
        "every line outside {key} is preserved"
    );
}

#[test]
fn missing_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = read_options(&dir.path().join("options.txt")).unwrap_err();
    assert_eq!(err.code(), "options_txt");
    assert!(
        err.to_string().contains("launch Minecraft once"),
        "error asks the user to launch the game once: {err}"
    );
}

#[test]
fn invalid_utf8_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("options.txt");
    std::fs::write(&path, [0xff, 0xfe, b'r']).unwrap();

    let err = read_options(&path).unwrap_err();
    assert_eq!(err.code(), "options_txt");
    assert!(
        err.to_string().contains("launch Minecraft once"),
        "error asks the user to launch the game once: {err}"
    );
}

#[test]
fn only_foreign_entries_round_trip() {
    let text = "version:4556\nresourcePacks:[\"vanilla\",\"fabric\"]\nlang:en_us\n";
    let (_dir, path) = temp_options(text);

    let parsed = read_options(&path).unwrap();
    assert_eq!(parsed.resource_packs(), ["vanilla", "fabric"]);
    write_options(
        &path,
        &parsed.render(parsed.resource_packs(), parsed.incompatible_packs()),
    )
    .unwrap();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn unicode_names_round_trip() {
    let text = "resourcePacks:[\"file/Ünïcode-\\u65e5\\u672c\\u8a9e.zip\",\"file/café.zip\"]\n";
    let (_dir, path) = temp_options(text);

    let parsed = read_options(&path).unwrap();
    assert_eq!(
        parsed.resource_packs(),
        ["file/Ünïcode-日本語.zip", "file/café.zip"]
    );
    let rendered = parsed.render(parsed.resource_packs(), parsed.incompatible_packs());
    let again = OptionsTxt::parse(&rendered).unwrap();
    assert_eq!(again.resource_packs(), parsed.resource_packs());
}

#[test]
fn quotes_and_backslashes_round_trip() {
    let text = "resourcePacks:[\"file/say \\\"hi\\\".zip\",\"file/back\\\\slash.zip\"]\n";
    let (_dir, path) = temp_options(text);

    let parsed = read_options(&path).unwrap();
    assert_eq!(
        parsed.resource_packs(),
        ["file/say \"hi\".zip", "file/back\\slash.zip"]
    );
    let rendered = parsed.render(parsed.resource_packs(), parsed.incompatible_packs());
    assert_eq!(
        rendered, text,
        "serde_json serialization matches the source"
    );
}

#[test]
fn duplicate_array_entries_are_preserved() {
    let text = "resourcePacks:[\"file/a.zip\",\"vanilla\",\"file/a.zip\"]\n";
    let (_dir, path) = temp_options(text);

    let parsed = read_options(&path).unwrap();
    assert_eq!(
        parsed.resource_packs(),
        ["file/a.zip", "vanilla", "file/a.zip"]
    );
    write_options(
        &path,
        &parsed.render(parsed.resource_packs(), parsed.incompatible_packs()),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn missing_trailing_newline_is_preserved() {
    let text = "lang:en_us\nresourcePacks:[\"vanilla\"]";
    let (_dir, path) = temp_options(text);

    let parsed = read_options(&path).unwrap();
    write_options(
        &path,
        &parsed.render(parsed.resource_packs(), parsed.incompatible_packs()),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn crlf_line_endings_are_preserved_per_line() {
    let text = "version:4556\r\nresourcePacks:[\"vanilla\"]\r\nlang:en_us\nlast:line";
    let (_dir, path) = temp_options(text);

    let parsed = read_options(&path).unwrap();
    let mut resource = parsed.resource_packs().to_vec();
    resource.push("file/new.zip".to_string());
    let rendered = parsed.render(&resource, parsed.incompatible_packs());

    assert_eq!(
        rendered,
        "version:4556\r\nresourcePacks:[\"vanilla\",\"file/new.zip\"]\r\nlang:en_us\nlast:line"
    );
}

#[test]
fn malformed_json_is_an_error() {
    let text = "resourcePacks:[\"vanilla\"\n";
    let parsed = OptionsTxt::parse(text).unwrap_err();
    assert_eq!(parsed.code(), "options_txt");
    assert!(parsed.to_string().contains("resourcePacks"), "{parsed}");
}

#[test]
fn non_array_value_is_an_error() {
    let parsed = OptionsTxt::parse("resourcePacks:12\n").unwrap_err();
    assert_eq!(parsed.code(), "options_txt");
    assert!(parsed.to_string().contains("resourcePacks"), "{parsed}");
}

#[test]
fn missing_resource_packs_is_an_error() {
    let parsed = OptionsTxt::parse("lang:en_us\n").unwrap_err();
    assert_eq!(parsed.code(), "options_txt");
    assert!(parsed.to_string().contains("resourcePacks"), "{parsed}");
}

#[test]
fn duplicate_target_keys_are_an_error() {
    let text = "resourcePacks:[\"vanilla\"]\nresourcePacks:[\"fabric\"]\n";
    let parsed = OptionsTxt::parse(text).unwrap_err();
    assert_eq!(parsed.code(), "options_txt");
    assert!(parsed.to_string().contains("resourcePacks"), "{parsed}");

    let text =
        "resourcePacks:[\"vanilla\"]\nincompatibleResourcePacks:[]\nincompatibleResourcePacks:[]\n";
    let parsed = OptionsTxt::parse(text).unwrap_err();
    assert_eq!(parsed.code(), "options_txt");
}

#[test]
fn missing_incompatible_line_is_an_empty_list() {
    let text = "resourcePacks:[\"vanilla\"]\nlang:en_us\n";
    let (_dir, path) = temp_options(text);

    let parsed = read_options(&path).unwrap();
    assert!(parsed.incompatible_packs().is_empty());

    let mut resource = parsed.resource_packs().to_vec();
    resource.push("file/new.zip".to_string());
    let rendered = parsed.render(&resource, parsed.incompatible_packs());
    assert_eq!(
        rendered, "resourcePacks:[\"vanilla\",\"file/new.zip\"]\nlang:en_us\n",
        "the missing incompatibleResourcePacks line is not added"
    );
}

#[test]
fn external_edit_is_preserved_on_the_next_write() {
    let text = "version:4556\nresourcePacks:[\"vanilla\"]\nlang:en_us\n";
    let (_dir, path) = temp_options(text);

    // The game rewrites an unrelated line while the app is open.
    let edited = "version:4556\nresourcePacks:[\"vanilla\"]\nlang:de_de\nrenderDistance:12\n";
    std::fs::write(&path, edited).unwrap();

    let latest = read_options(&path).unwrap();
    let mut resource = latest.resource_packs().to_vec();
    resource.push("file/new.zip".to_string());
    write_options(
        &path,
        &latest.render(&resource, latest.incompatible_packs()),
    )
    .unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("lang:de_de"),
        "external edit survives: {after}"
    );
    assert!(
        after.contains("renderDistance:12"),
        "new line survives: {after}"
    );
    assert_only_target_line_changed(edited, &after, "resourcePacks");
    assert_eq!(
        line_with_key(&after, "resourcePacks"),
        "resourcePacks:[\"vanilla\",\"file/new.zip\"]"
    );
}

#[test]
fn toggles_change_only_the_resource_packs_line() {
    let text = "version:4556\n\
        resourcePacks:[\"vanilla\",\"fabric\",\"file/a.zip\",\"file/a.zip\"]\n\
        incompatibleResourcePacks:[\"file/a.zip\"]\n\
        lang:en_us\n";
    let (_dir, path) = temp_options(text);

    let parsed = read_options(&path).unwrap();
    let mut resource = parsed.resource_packs().to_vec();
    let mut incompatible = parsed.incompatible_packs().to_vec();
    let changed = resource_stack::disable(&mut resource, &mut incompatible, "a.zip");
    assert!(changed);
    assert_eq!(resource, ["vanilla", "fabric"]);
    assert!(incompatible.is_empty());

    let rendered = parsed.render(&resource, &incompatible);
    write_options(&path, &rendered).unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    let outside = |content: &str| -> Vec<String> {
        content
            .lines()
            .filter(|line| {
                !line.starts_with("resourcePacks:")
                    && !line.starts_with("incompatibleResourcePacks:")
            })
            .map(str::to_string)
            .collect()
    };
    assert_eq!(
        outside(text),
        outside(&after),
        "every line outside the two target keys is preserved"
    );
    assert_eq!(
        line_with_key(&after, "resourcePacks"),
        "resourcePacks:[\"vanilla\",\"fabric\"]"
    );
    assert_eq!(
        line_with_key(&after, "incompatibleResourcePacks"),
        "incompatibleResourcePacks:[]"
    );
}

#[test]
fn real_copy_round_trips_byte_identical() {
    let Some(source) = real_copy() else {
        eprintln!("skipping: the real options.txt copy is not in {REAL_COPY}");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("options.txt");
    std::fs::copy(&source, &path).unwrap();

    let original = std::fs::read(&path).unwrap();
    let parsed = read_options(&path).unwrap();
    assert!(
        !parsed.resource_packs().is_empty(),
        "the real file has a non-empty resourcePacks"
    );

    write_options(
        &path,
        &parsed.render(parsed.resource_packs(), parsed.incompatible_packs()),
    )
    .unwrap();

    let after = std::fs::read(&path).unwrap();
    assert_eq!(
        original, after,
        "the real options.txt round-trips with no mutation"
    );
}

#[test]
fn real_copy_toggle_changes_only_the_resource_packs_line() {
    let Some(source) = real_copy() else {
        eprintln!("skipping: the real options.txt copy is not in {REAL_COPY}");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("options.txt");
    std::fs::copy(&source, &path).unwrap();

    let original = String::from_utf8(std::fs::read(&path).unwrap()).unwrap();
    let parsed = read_options(&path).unwrap();
    let real_entry = parsed
        .resource_packs()
        .iter()
        .find(|entry| entry.starts_with("file/"))
        .cloned()
        .expect("the real file has a file/ entry");
    let filename = real_entry.strip_prefix("file/").unwrap().to_string();

    let mut resource = parsed.resource_packs().to_vec();
    let mut incompatible = parsed.incompatible_packs().to_vec();
    let changed = resource_stack::disable(&mut resource, &mut incompatible, &filename);
    assert!(changed, "{filename} is present in the real file");

    write_options(&path, &parsed.render(&resource, &incompatible)).unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert_only_target_line_changed(&original, &after, "resourcePacks");
    assert!(!line_with_key(&after, "resourcePacks").contains(&real_entry));
}

#[test]
fn write_never_creates_options_txt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("options.txt");

    let err = write_options(&path, "resourcePacks:[]\n").unwrap_err();
    assert_eq!(err.code(), "options_txt");
    assert!(!path.exists(), "a failed write leaves no file behind");
}

#[test]
fn failed_parse_leaves_the_file_untouched() {
    let text = "resourcePacks:[oops]\n";
    let (_dir, path) = temp_options(text);

    assert!(read_options(&path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn renders_keys_in_place_with_original_spacing_keys() {
    let text = "version:4556\nresourcePacks:[\"vanilla\"]\n";
    let (_dir, path) = temp_options(text);

    let parsed = read_options(&path).unwrap();
    let resource = vec!["fabric".to_string()];
    let rendered = parsed.render(&resource, parsed.incompatible_packs());
    assert_eq!(rendered, "version:4556\nresourcePacks:[\"fabric\"]\n");
}

#[test]
fn temp_files_are_cleaned_from_the_options_dir() {
    let text = "resourcePacks:[\"vanilla\"]\n";
    let (dir, path) = temp_options(text);
    let parsed = read_options(&path).unwrap();
    write_options(
        &path,
        &parsed.render(parsed.resource_packs(), parsed.incompatible_packs()),
    )
    .unwrap();

    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, [std::ffi::OsString::from("options.txt")]);
}

fn _assert_path_send(_: &Path) {}
