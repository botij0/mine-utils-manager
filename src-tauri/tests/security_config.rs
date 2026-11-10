use serde_json::Value;

#[test]
fn shell_restricts_content_and_permissions() {
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let capability: Value =
        serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
    let csp = config["app"]["security"]["csp"]
        .as_str()
        .expect("CSP must be set");
    assert!(csp
        .split(';')
        .any(|directive| directive.trim() == "default-src 'self'"));
    let permissions = capability["permissions"].as_array().unwrap();
    assert!(permissions
        .iter()
        .any(|permission| permission == "dialog:allow-open"));
    assert!(!permissions.iter().any(|permission| {
        let name = permission.as_str().unwrap();
        name == "dialog:default" || name.starts_with("opener:")
    }));
}
