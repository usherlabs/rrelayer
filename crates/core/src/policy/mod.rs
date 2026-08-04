//! Policy enforcement primitives: IP allowlisting and request signature
//! verification. Used by `NetworkPermissionsConfig` to gate access to relayer
//! endpoints when the rrelayer is fronting an Appsmith-style API.

pub mod ip_allowlist;
pub mod signature;

pub use ip_allowlist::{ip_allowed, IpAllowlistError};
pub use signature::{verify_request_signature, SignatureVerificationError};
