pub mod error;
pub mod loop_scheduler;
pub mod loop_types;
pub mod process_manager;
pub mod protocol;

pub use error::{RalphError, Result};
pub use loop_scheduler::LoopScheduler;
pub use loop_types::{LoopDispatch, LoopType, QueuedLoop};
pub use process_manager::{RalphProcessManager, RunId};
pub use protocol::{NdjsonCodec, WorkerCommand, WorkerEvent};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
