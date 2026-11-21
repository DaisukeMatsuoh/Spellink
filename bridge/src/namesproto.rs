//! What the bridge (or the relay) and the name service say to each other (ADR 0017,
//! docs/spec/interfaces.md I28–I29): an Ed25519 identity, an ID derived from its public key, and
//! signed requests. Nobody needs an account: the ID is a hash of the public key, so nobody else
//! can claim it.

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::Ipv4Addr;
use std::path::Path;

pub const PROTOCOL: &str = "spellink-names/1";
/// How far a request's time may be from the server's clock.
pub const MAX_SKEW_SECS: i64 = 300;
pub const MAX_BODY: usize = 4096;

pub fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub fn unb64(s: &str) -> Option<Vec<u8>> {
    B64.decode(s).ok()
}

/// The ID of a public key: the first 16 bytes of its SHA-256, in lower-case hex (32 characters).
pub fn id_of(public_key: &[u8]) -> String {
    hex::encode(&Sha256::digest(public_key)[..16])
}

pub fn valid_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The value of an ACME DNS-01 TXT record: the base64url of a SHA-256 (43 characters).
pub fn valid_txt_value(s: &str) -> bool {
    s.len() == 43 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// 16 random bytes, base64url (22 characters).
pub fn valid_nonce(s: &str) -> bool {
    s.len() == 22 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// What a request signs: the protocol, what it asks for, which ID, the value, when, and a nonce.
pub fn signing_input(op: &str, id: &str, value: &str, time: i64, nonce: &str) -> Vec<u8> {
    format!("{PROTOCOL}\n{op}\n{id}\n{value}\n{time}\n{nonce}").into_bytes()
}

pub fn verify(public_key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    UnparsedPublicKey::new(&ED25519, public_key).verify(message, signature).is_ok()
}

/// An address that may be published in public DNS for the LAN name: private IPv4 only.
pub fn publishable_ip(s: &str) -> Option<Ipv4Addr> {
    s.parse::<Ipv4Addr>().ok().filter(|ip| ip.is_private())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterReq {
    pub id: String,
    pub public_key: String,
    /// A private IPv4 address, or empty for a service that only needs a TXT record.
    pub ip: String,
    pub time: i64,
    pub nonce: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcmeReq {
    pub id: String,
    /// "set" or "clear".
    pub op: String,
    pub value: String,
    pub time: i64,
    pub nonce: String,
    pub signature: String,
}

pub fn acme_op(op: &str) -> Option<&'static str> {
    match op {
        "set" => Some("acme-set"),
        "clear" => Some("acme-clear"),
        _ => None,
    }
}

/// The key pair that names this bridge (or relay) to the name service. It never leaves the
/// machine; the file is private (0600).
pub struct Identity {
    pair: Ed25519KeyPair,
    id: String,
}

impl Identity {
    fn from_pkcs8(bytes: &[u8]) -> Result<Identity, String> {
        let pair = Ed25519KeyPair::from_pkcs8(bytes).map_err(|e| format!("identity key: {e}"))?;
        let id = id_of(pair.public_key().as_ref());
        Ok(Identity { pair, id })
    }

    /// Reads `<dir>/names-identity.pk8`, or creates it.
    pub fn load_or_create(dir: &Path) -> Result<Identity, String> {
        let path = dir.join("names-identity.pk8");
        match std::fs::read(&path) {
            Ok(bytes) => return Identity::from_pkcs8(&bytes).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|e| format!("identity key: {e}"))?;
        crate::util::create_private_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        crate::util::write_private_atomic(&path, doc.as_ref()).map_err(|e| format!("{}: {e}", path.display()))?;
        // read back: success is confirmed by the result, not by the write
        let again = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if again != doc.as_ref() {
            return Err(format!("{}: reading the new identity back gave other bytes", path.display()));
        }
        Identity::from_pkcs8(&again)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn public_key(&self) -> Vec<u8> {
        self.pair.public_key().as_ref().to_vec()
    }

    fn sign(&self, message: &[u8]) -> String {
        b64(self.pair.sign(message).as_ref())
    }

    fn nonce() -> String {
        use rand::RngCore;
        let mut n = [0u8; 16];
        rand::rng().fill_bytes(&mut n);
        b64(&n)
    }

    pub fn register(&self, ip: &str, time: i64) -> RegisterReq {
        let nonce = Identity::nonce();
        let signature = self.sign(&signing_input("register", &self.id, ip, time, &nonce));
        RegisterReq { id: self.id.clone(), public_key: b64(&self.public_key()), ip: ip.to_string(), time, nonce, signature }
    }

    pub fn acme(&self, op: &str, value: &str, time: i64) -> AcmeReq {
        let nonce = Identity::nonce();
        let signed = acme_op(op).unwrap_or("acme-set");
        let signature = self.sign(&signing_input(signed, &self.id, value, time, &nonce));
        AcmeReq { id: self.id.clone(), op: op.to_string(), value: value.to_string(), time, nonce, signature }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_is_the_hash_of_the_public_key_and_a_signature_proves_it() {
        let dir = tempfile::tempdir().unwrap();
        let me = Identity::load_or_create(dir.path()).unwrap();
        assert!(valid_id(me.id()));
        assert_eq!(me.id(), id_of(&me.public_key()));
        let req = me.register("192.168.1.5", 1000);
        let msg = signing_input("register", &req.id, &req.ip, req.time, &req.nonce);
        assert!(verify(&me.public_key(), &msg, &unb64(&req.signature).unwrap()));
        // any change of what was signed breaks it
        let other = signing_input("register", &req.id, "192.168.1.6", req.time, &req.nonce);
        assert!(!verify(&me.public_key(), &other, &unb64(&req.signature).unwrap()));
        // the same file gives the same identity
        assert_eq!(Identity::load_or_create(dir.path()).unwrap().id(), me.id());
    }

    #[test]
    fn a_signature_for_one_operation_is_not_valid_for_another() {
        let dir = tempfile::tempdir().unwrap();
        let me = Identity::load_or_create(dir.path()).unwrap();
        let v = "a".repeat(43);
        let set = me.acme("set", &v, 5);
        let as_clear = signing_input("acme-clear", &set.id, &set.value, set.time, &set.nonce);
        assert!(!verify(&me.public_key(), &as_clear, &unb64(&set.signature).unwrap()));
    }

    #[test]
    fn shapes() {
        assert!(valid_id(&"0123456789abcdef".repeat(2)));
        assert!(!valid_id(&"0123456789ABCDEF".repeat(2)), "lower case only: the ID becomes part of a DNS name");
        assert!(!valid_id("short"));
        assert!(valid_txt_value(&"A".repeat(43)) && !valid_txt_value(&"A".repeat(44)) && !valid_txt_value(&format!("{}=", "A".repeat(42))));
        assert!(valid_nonce(&"A".repeat(22)) && !valid_nonce("x"));
        assert_eq!(publishable_ip("192.168.0.4"), Some(Ipv4Addr::new(192, 168, 0, 4)));
        for bad in ["8.8.8.8", "127.0.0.1", "169.254.1.1", "::1", "", "10.0.0.1/8", "300.1.1.1"] {
            assert_eq!(publishable_ip(bad), None, "{bad}");
        }
    }
}
