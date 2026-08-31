//! Browser engine spike, multi-process supervisor, and capability provider
//! for Worldline M1.1.
//!
//! Provides the executable spike harness, out-of-process engine simulation,
//! deterministic local test page fixtures, crash isolation tests, and
//! empirical measurements for the M1.2 engine selection decision.

#![forbid(unsafe_code)]

pub mod engine;
pub mod harness;
pub mod provider;

pub use engine::{ContextState, FormElementState, PageState, SpikeEngineSupervisor};
pub use harness::BrowserSpikeFixture;
pub use provider::{SpikeBrowserPlugin, browser_capability};
