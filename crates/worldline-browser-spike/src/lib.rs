//! Browser engine spike, multi-process supervisor, and capability provider
//! for Worldline M1.1.
//!
//! Provides the executable real Chromium spike process manager, minimal CDP client,
//! in-memory reference engine model, deterministic local test page fixtures,
//! confused-deputy security enforcement, M0.4 event emission, and empirical measurements.

#![forbid(unsafe_code)]

pub mod cdp;
pub mod chromium;
pub mod engine;
pub mod harness;
pub mod provider;

pub use cdp::CdpSession;
pub use chromium::{ChromiumBinaryInfo, ChromiumEngineSupervisor, discover_chromium_binary};
pub use engine::{ContextState, FormElementState, PageState, ReferenceBrowserSupervisor};
pub use harness::BrowserSpikeFixture;
pub use provider::{SpikeBrowserPlugin, browser_capability};
