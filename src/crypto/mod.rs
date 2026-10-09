pub mod secure_memory;
pub mod signer;

pub use secure_memory::ProtectedKey;
pub use signer::{create_signer_from_protected, get_address_from_protected};
