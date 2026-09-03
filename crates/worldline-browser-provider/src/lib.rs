//! Browser engine provider core implementation and reference backend.

pub mod backend;
pub mod core;
pub mod diagnostics;
pub mod reference;
pub mod request_policy;

pub use backend::BrowserBackend;
pub use core::{BrowserProviderCore, ProviderBudgetLimits};
pub use diagnostics::{
    DiagnosticSink, MemoryDiagnosticSink, ProviderConsoleLogLevel, ProviderDiagnosticEvent,
    ProviderNetworkRequestStatus, SharedDiagnosticSink,
};
pub use reference::ReferenceBrowserBackend;
pub use request_policy::{
    RequestPolicyBroker, RequestPolicyBrokerError, RequestPolicyBrokerLimits, RequestPolicyCaller,
    RequestPolicyCancellation, RequestPolicyEvaluator, RequestPolicyEvaluatorError,
    RequestPolicyTransport, RequestPolicyTransportError,
};
