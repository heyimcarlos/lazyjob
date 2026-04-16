pub mod error;
pub mod process_manager;
pub mod protocol;

pub use error::{RalphError, Result};
pub use process_manager::{RalphProcessManager, RunId};
pub use protocol::{NdjsonCodec, WorkerCommand, WorkerEvent};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
