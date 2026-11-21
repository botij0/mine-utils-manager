use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{FnArg, Item, Token, Type};

#[derive(Default)]
struct Handlers(Vec<syn::Macro>);

impl<'ast> Visit<'ast> for Handlers {
    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        if invocation
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "generate_handler")
        {
            self.0.push(invocation.clone());
        }
    }
}

pub fn generate() -> Result<(), Box<dyn std::error::Error>> {
    let source_dir = Path::new("src/commands");
    println!("cargo:rerun-if-changed={}", source_dir.display());
    println!("cargo:rerun-if-changed=src/lib.rs");

    let mut handlers = Handlers::default();
    handlers.visit_file(&syn::parse_file(&fs::read_to_string("src/lib.rs")?)?);
    let [handler] = handlers.0.as_slice() else {
        return Err("expected one Tauri command registration in src/lib.rs".into());
    };
    let mut registered = BTreeSet::new();
    for path in handler.parse_body_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)? {
        let segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        registered.insert(format!("crate::{}", segments.join("::")));
    }

    let sources = command_sources(source_dir)?;
    let mut checks = String::from(
        "#[test]\nfn command_wire_types_are_exported() {\n    let exports = WireExports::collect();\n",
    );
    for path in sources {
        let module = path.file_stem().unwrap().to_str().unwrap();
        let module = if module == "mod" {
            "crate::commands".to_owned()
        } else {
            format!("crate::commands::{module}")
        };
        write_checks(
            &mut checks,
            &mut registered,
            &module,
            &syn::parse_file(&fs::read_to_string(path)?)?.items,
        )?;
    }
    if !registered.is_empty() {
        return Err(
            format!("cannot find wire signatures for registered commands: {registered:?}").into(),
        );
    }
    checks.push_str("}\n");
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    fs::write(output_dir.join("command_wire_checks.rs"), checks)?;
    Ok(())
}

fn command_sources(source_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut sources = fs::read_dir(source_dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    sources.retain(|path| path.extension().and_then(|extension| extension.to_str()) == Some("rs"));
    sources.sort();
    Ok(sources)
}

fn write_checks(
    checks: &mut String,
    registered: &mut BTreeSet<String>,
    module: &str,
    items: &[Item],
) -> std::fmt::Result {
    for item in items {
        let Item::Fn(function) = item else {
            if let Item::Mod(nested) = item {
                if let Some((_, items)) = &nested.content {
                    write_checks(
                        checks,
                        registered,
                        &format!("{module}::{}", nested.ident),
                        items,
                    )?;
                }
            }
            continue;
        };
        let command = format!("{module}::{}", function.sig.ident);
        if !registered.remove(&command) {
            continue;
        }

        let mut args = String::new();
        let mut state = String::new();
        let mut state_types = String::new();
        let mut call_args = Vec::new();
        for (index, argument) in function.sig.inputs.iter().enumerate() {
            let FnArg::Typed(argument) = argument else {
                panic!("unexpected self argument in {command}");
            };
            let name = format!("arg_{index}");
            if type_segment(&argument.ty).is_some_and(|segment| segment.ident == "State") {
                write!(state, "{name}, ")?;
                state_types.push_str("tauri::State<'static, _>, ");
            } else if type_segment(&argument.ty).is_some_and(|segment| segment.ident == "AppHandle")
            {
                write!(state, "{name}, ")?;
                state_types.push_str("tauri::AppHandle, ");
            } else {
                write!(args, "{name}, ")?;
            }
            call_args.push(name);
        }

        let await_call = if function.sig.asyncness.is_some() {
            ".await"
        } else {
            ""
        };
        writeln!(
            checks,
            "    assert_signature(&exports, {command:?}, |({args}), ({state}): ({state_types})| async move {{ {command}({}){await_call} }});",
            call_args.join(", "),
        )?;
    }
    Ok(())
}

fn type_segment(ty: &Type) -> Option<&syn::PathSegment> {
    let Type::Path(path) = ty else { return None };
    path.path.segments.last()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_sources_do_not_include_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("top.rs"), "").unwrap();
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::write(dir.path().join("nested/command.rs"), "").unwrap();
        assert_eq!(
            command_sources(dir.path()).unwrap(),
            vec![dir.path().join("top.rs")]
        );
    }

    #[test]
    fn injected_app_handle_is_not_a_wire_argument() {
        let source = syn::parse_file("fn probe(app: tauri::AppHandle, name: String) {}").unwrap();
        let mut registered = BTreeSet::from(["crate::commands::probe".to_owned()]);
        let mut checks = String::new();
        write_checks(
            &mut checks,
            &mut registered,
            "crate::commands",
            &source.items,
        )
        .unwrap();
        assert!(
            checks.contains("|(arg_1, ), (arg_0, ): (tauri::AppHandle, )|"),
            "{checks}"
        );
    }
}
