use anyhow::Result;
use ed25519_dalek::Verifier;
use ed25519_dalek::KEYPAIR_LENGTH;
use std::collections::BTreeMap;
use std::fmt::Display;

use ed25519_dalek::PUBLIC_KEY_LENGTH;

use ed25519_dalek::Signer;
use serde::{Deserialize, Serialize};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Signature verify failed: {0}")]
    ParseError(#[from] ed25519_dalek::SignatureError),
}

#[derive(PartialEq, Eq, Serialize, Clone, Copy, Deserialize, Default, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    pub fn new(data: [u8; 32]) -> Self {
        Self(data)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        let mut s = String::new();
        let table = b"0123456789abcdef";
        for &b in self.0.iter() {
            s.push(table[(b >> 4) as usize] as char);
            s.push(table[(b & 0xf) as usize] as char);
        }
        s
    }

    pub fn display(&self) -> String {
        self.to_hex().chars().take(8).collect::<String>()
    }
}

impl Display for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

impl std::fmt::Debug for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

impl From<[u8; 32]> for Digest {
    fn from(data: [u8; 32]) -> Self {
        Self::new(data)
    }
}

impl From<blake3::Hash> for Digest {
    fn from(value: blake3::Hash) -> Self {
        Digest::from(<[u8; 32]>::from(value))
    }
}

pub(crate) fn hash(data: &[u8]) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(data);
    let hash = hasher.finalize();
    Digest::from(<[u8; 32]>::from(hash))
}

#[derive(PartialEq, Eq, Hash, Clone, PartialOrd, Ord, Copy, Default)]
pub struct PublicKey([u8; PUBLIC_KEY_LENGTH]);

impl PublicKey {
    pub fn encode(&self) -> String {
        base64::encode(self.0)
    }

    pub fn decode(s: &str) -> Result<Self, String> {
        let bytes = base64::decode(s).map_err(|e| e.to_string())?;
        if bytes.len() != PUBLIC_KEY_LENGTH {
            return Err(format!(
                "invalid public key length: {} (expected {})",
                bytes.len(),
                PUBLIC_KEY_LENGTH
            ));
        }
        let mut key = [0; PUBLIC_KEY_LENGTH];
        key.copy_from_slice(&bytes);
        Ok(PublicKey(key))
    }

    fn to_base32(self) -> String {
        base32::encode(base32::Alphabet::Crockford, &self.0)
    }

    pub fn display(&self) -> String {
        self.to_base32().chars().take(8).collect::<String>()
    }

    pub fn verify(&self, msg: &Digest, signature: &Signature) -> Result<(), CryptoError> {
        let public_key = ed25519_dalek::PublicKey::from_bytes(&self.0)?;
        let signature = ed25519_dalek::Signature::from_bytes(&signature.flatten())?;
        public_key.verify(msg.as_bytes(), &signature)?;
        Ok(())
    }
}

impl Display for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

impl Serialize for PublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.encode())
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let key = PublicKey::decode(&s).map_err(serde::de::Error::custom)?;
        Ok(key)
    }
}

impl From<[u8; PUBLIC_KEY_LENGTH]> for PublicKey {
    fn from(data: [u8; 32]) -> Self {
        Self(data)
    }
}

// WARN: This is not a secure solution, but it's good enough for prototypes.
#[derive(Hash, PartialEq, Clone)]
pub struct Keypair([u8; KEYPAIR_LENGTH]);

impl Keypair {
    pub fn encode(&self) -> String {
        base64::encode(self.0)
    }

    pub fn decode(s: &str) -> Result<Self, String> {
        let bytes = base64::decode(s).map_err(|e| e.to_string())?;
        if bytes.len() != KEYPAIR_LENGTH {
            return Err(format!(
                "invalid private key length: {} (expected {})",
                bytes.len(),
                KEYPAIR_LENGTH
            ));
        }
        let mut key = [0; KEYPAIR_LENGTH];
        key.copy_from_slice(&bytes);
        Ok(Keypair(key))
    }

    pub fn display(&self) -> String {
        self.encode().chars().take(2).collect::<String>()
    }

    pub fn sign(&self, data: &Digest) -> Signature {
        let keypair = ed25519_dalek::Keypair::from_bytes(&self.0).unwrap();
        let signature = keypair.sign(data.as_bytes());
        Signature::from(signature)
    }
}

impl Display for Keypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}...", self.display())
    }
}

impl std::fmt::Debug for Keypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}...", self.display())
    }
}

impl Serialize for Keypair {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.encode())
    }
}

impl<'de> Deserialize<'de> for Keypair {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let key = Keypair::decode(&s).map_err(serde::de::Error::custom)?;
        Ok(key)
    }
}

impl From<[u8; KEYPAIR_LENGTH]> for Keypair {
    fn from(data: [u8; KEYPAIR_LENGTH]) -> Self {
        Self(data)
    }
}

pub fn generate_keypair() -> (PublicKey, Keypair) {
    let mut rng = rand::thread_rng();
    // let mut seed = [0u8; 32];
    // seed[0..8].copy_from_slice(&id.to_be_bytes());
    // rng.fill_bytes(&mut seed);
    let keypair = ed25519_dalek::Keypair::generate(&mut rng);
    let public_key = PublicKey::from(keypair.public.to_bytes());
    let private_key = Keypair::from(keypair.to_bytes());
    (public_key, private_key)
}

/// Generate a vector of keypairs, sorted by public key.
pub fn generate_keypairs(number: usize) -> Vec<(PublicKey, Keypair)> {
    let mut map = BTreeMap::new();
    let mut keypairs = Vec::with_capacity(number);
    for _ in 0..number {
        let (public_key, private_key) = generate_keypair();
        map.insert(public_key, private_key);
    }

    map.into_iter().for_each(|(public_key, private_key)| {
        keypairs.push((public_key, private_key));
    });
    keypairs
}

/// Represents an ed25519 signature.
/// Code borrowed from https://github.com/asonnino/hotstuff
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Signature {
    part1: [u8; 32],
    part2: [u8; 32],
}

impl Signature {
    #[allow(dead_code)]
    pub fn new(digest: &Digest, secret: &Keypair) -> Self {
        let keypair =
            ed25519_dalek::Keypair::from_bytes(&secret.0).expect("Unable to load secret key");
        let sig = keypair.sign(&digest.0).to_bytes();
        let part1 = sig[..32].try_into().expect("Unexpected signature length");
        let part2 = sig[32..64].try_into().expect("Unexpected signature length");
        Signature { part1, part2 }
    }

    fn flatten(&self) -> [u8; 64] {
        [self.part1, self.part2]
            .concat()
            .try_into()
            .expect("Unexpected signature length")
    }

    #[allow(dead_code)]
    pub fn verify(&self, digest: &Digest, public_key: &PublicKey) -> Result<()> {
        let signature = ed25519_dalek::Signature::from_bytes(&self.flatten())?;
        let key = ed25519_dalek::PublicKey::from_bytes(&public_key.0)?;
        key.verify_strict(&digest.0, &signature)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn verify_batch<'a, I>(digest: &Digest, votes: I) -> Result<()>
    where
        I: IntoIterator<Item = &'a (PublicKey, Signature)>,
    {
        let mut messages: Vec<&[u8]> = Vec::new();
        let mut signatures: Vec<ed25519_dalek::Signature> = Vec::new();
        let mut keys: Vec<ed25519_dalek::PublicKey> = Vec::new();
        for (key, sig) in votes.into_iter() {
            messages.push(&digest.0[..]);
            signatures.push(ed25519_dalek::Signature::from_bytes(&sig.flatten())?);
            keys.push(ed25519_dalek::PublicKey::from_bytes(&key.0)?);
        }
        ed25519_dalek::verify_batch(&messages[..], &signatures[..], &keys[..])?;
        Ok(())
    }
}

impl From<ed25519_dalek::Signature> for Signature {
    fn from(sig: ed25519_dalek::Signature) -> Self {
        let sig = sig.to_bytes();
        let part1 = sig[..32].try_into().expect("Unexpected signature length");
        let part2 = sig[32..64].try_into().expect("Unexpected signature length");
        Signature { part1, part2 }
    }
}

#[cfg(test)]
mod test {
    use ed25519_dalek::{Signer, Verifier};

    use super::*;

    #[test]
    pub fn test() {
        let hash = blake3::hash(b"hello world");

        let digest = Digest::from(*hash.as_bytes());

        assert_eq!(
            digest.to_hex(),
            "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"
        );

        assert_eq!(digest.display(), "d74981ef");
    }

    use rand::rngs::OsRng;

    #[test]
    pub fn test_ed25519() {
        let mut csprng = OsRng {};
        let keypair = ed25519_dalek::Keypair::generate(&mut csprng);
        let public_key = keypair.public;
        let message = b"hello world";
        let signature = keypair.sign(message);
        assert!(public_key.verify(message, &signature).is_ok());
    }
    #[test]
    pub fn test_encode_decode() {
        let (public_key, private_key) = generate_keypair();
        let public_key_str = public_key.encode();
        let private_key_str = private_key.encode();
        let public_key2 = PublicKey::decode(&public_key_str).unwrap();
        let private_key2 = Keypair::decode(&private_key_str).unwrap();
        assert_eq!(public_key, public_key2);
        assert_eq!(private_key, private_key2);
    }
}
