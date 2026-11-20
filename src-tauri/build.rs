#[path = "build/wire_checks.rs"]
mod wire_checks;

fn main() {
    wire_checks::generate().expect("generate command wire type checks");
    tauri_build::build();
}
