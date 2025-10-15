pub mod error;
pub mod escrow;
pub mod finality_adapter;
pub mod lifecycle;
pub mod message_trait;
pub mod network_metadata;
pub mod parameter_store;
pub mod params;
pub mod security;
pub mod types;

#[cfg(feature = "http-metadata")]
pub mod network_metadata_http;

#[cfg(feature = "http-metadata")]
pub mod cert_pinning;

pub use error::{AppError, Result};
pub use escrow::{EscrowManager, EscrowType};
pub use finality_adapter::{FinalityAdapter, FinalityPolicy, FinalityStatus, TimelockConfig};
pub use lifecycle::{
    EscrowAction, LifecycleEvent, LifecycleManager, LifecycleState, StateTransition,
};
pub use message_trait::{AppMessage, AppMessageType, RepRequirements};
pub use network_metadata::{NetworkMetadataRegistry, NodeNetworkInfo};

#[cfg(feature = "http-metadata")]
pub use network_metadata_http::{HttpMetadataConfig, HttpMetadataFetcher, MetadataResponse};
pub use parameter_store::{ParameterChange, ParameterMetadata, ParameterStore, ValidationRule};
pub use params::AppParams;
pub use security::{
    CertFingerprint, CertificatePinning, RequestSignature, SecurityAuditEvent, SecurityAuditLogger,
    SecuritySeverity,
};
pub use types::*;
