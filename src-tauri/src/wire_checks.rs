use std::future::Future;

use mine_utils_core::{error::AppErrorWire, wire_exports::WireExports, AppError};
use ts_rs::TS;

trait WireOutput<Payload> {
    fn assert_exported(exports: &WireExports, command: &str);
}

impl<Payload: TS + 'static> WireOutput<Payload> for Payload {
    fn assert_exported(exports: &WireExports, command: &str) {
        exports.assert_exported::<Payload>(command);
    }
}

impl<Payload: TS + 'static> WireOutput<Payload> for Result<Payload, AppError> {
    fn assert_exported(exports: &WireExports, command: &str) {
        exports.assert_exported::<Payload>(command);
        exports.assert_exported::<AppErrorWire>(command);
    }
}

// The closure gives Rust the actual command types without invoking the command.
fn assert_signature<Args, State, Output, Payload, Call>(
    exports: &WireExports,
    command: &str,
    _: impl FnOnce(Args, State) -> Call,
) where
    Args: TS + 'static,
    Output: WireOutput<Payload>,
    Call: Future<Output = Output>,
{
    exports.assert_exported::<Args>(command);
    Output::assert_exported(exports, command);
}

include!(concat!(env!("OUT_DIR"), "/command_wire_checks.rs"));
