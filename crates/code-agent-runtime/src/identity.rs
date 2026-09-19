use sha2::{Digest, Sha256};

pub fn content_id_for_bytes(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}
