use std::fmt::Display;

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Serialize, Clone, Copy, Deserialize, Default, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    pub fn new(data: [u8; 32]) -> Self {
        Self(data)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::new();
        let table = b"0123456789abcdef";
        for &b in self.0.iter() {
            s.push(table[(b >> 4) as usize] as char);
            s.push(table[(b & 0xf) as usize] as char);
        }
        s
    }

    pub fn to_base64(&self) -> String {
        base64::encode(self.0)
    }

    pub fn display(&self) -> String {
        self.to_hex().chars().take(8).collect::<String>()
    }

    pub fn genesis_digest() -> Digest {
        Digest::new([0; 32])
    }
}

impl Display for Digest {
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

#[cfg(test)]
mod test {
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
}
