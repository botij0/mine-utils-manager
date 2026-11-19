use serde_json::Value;

fn csp_directives(csp: &str) -> Vec<(&str, &str)> {
    csp.split(';')
        .map(str::trim)
        .filter(|directive| !directive.is_empty())
        .map(|directive| directive.split_once(' ').unwrap_or((directive, "")))
        .collect()
}

#[test]
fn shell_restricts_content_and_permissions() {
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let capability: Value =
        serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
    let security = &config["app"]["security"];
    let csp = security["csp"].as_str().expect("CSP must be set");
    assert_eq!(
        csp_directives(csp),
        vec![
            ("default-src", "'self'"),
            (
                "img-src",
                "'self' asset: http://asset.localhost https://cdn.modrinth.com data:",
            ),
            ("style-src", "'self' 'unsafe-inline'"),
            ("connect-src", "ipc: http://ipc.localhost"),
        ]
    );
    assert!(
        security.get("devCsp").is_none(),
        "devCsp must be absent. The desktop webview loads devUrl directly, and Tauri does not inject this field there."
    );
    let permissions = capability["permissions"].as_array().unwrap();
    assert!(permissions
        .iter()
        .any(|permission| permission == "dialog:allow-open"));
    assert!(!permissions.iter().any(|permission| {
        let name = permission.as_str().unwrap();
        name == "dialog:default" || name.starts_with("opener:")
    }));
}
