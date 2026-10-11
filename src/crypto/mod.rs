pub mod secure_memory;
pub mod signer;

pub use secure_memory::{harden_process, ProtectedKey};
pub use signer::{create_signer_from_protected, get_address_from_protected};
