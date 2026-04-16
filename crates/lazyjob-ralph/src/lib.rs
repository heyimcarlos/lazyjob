pub mod error;
pub mod protocol;

pub use error::{RalphError, Result};
pub use protocol::{NdjsonCodec, WorkerCommand, WorkerEvent};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
