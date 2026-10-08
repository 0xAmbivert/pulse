use alloy::primitives::{keccak256, eip191_hash_message};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;

#[test]
fn test_flashbots_signature_recovery() {
    let signer = PrivateKeySigner::random();
    let payload = r#"{"jsonrpc":"2.0","id":1,"method":"eth_sendPrivateTransaction","params":[]}"#;

    // Flashbots spec: hash = keccak256(body)
    let body_hash = keccak256(payload.as_bytes());
    // EIP-191 prefix: "\x19Ethereum Signed Message:\n32" + body_hash
    let eip191_hash = eip191_hash_message(body_hash);
    let signature = signer.sign_hash_sync(&eip191_hash).expect("sign failed");

    // Recover address from signature
    let recovered = signature.recover_address_from_prehash(&eip191_hash).expect("recover failed");
    assert_eq!(recovered, signer.address());
    assert_eq!(signature.as_bytes().len(), 65);
}
