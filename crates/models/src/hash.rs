use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A validated Stellar transaction hash.
///
/// Stellar transaction hashes are SHA-256 digests encoded as 64 hexadecimal
/// characters. Input is trimmed and normalized to lowercase, so
/// `"ABC…"` and `"abc…"` refer to the same transaction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TransactionHash(String);

/// Reasons a string is not a valid transaction hash.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidTransactionHash {
    #[error("transaction hash must not be empty")]
    Empty,
    #[error("transaction hash must be 64 hexadecimal characters, got {0} characters")]
    WrongLength(usize),
    #[error("transaction hash must contain only hexadecimal characters (0-9, a-f)")]
    NotHex,
}

impl TransactionHash {
    /// Number of hex characters in a transaction hash (32 bytes).
    pub const HEX_LEN: usize = 64;

    /// Parses and normalizes a transaction hash.
    pub fn parse(input: &str) -> Result<Self, InvalidTransactionHash> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(InvalidTransactionHash::Empty);
        }
        let len = trimmed.chars().count();
        if len != Self::HEX_LEN {
            return Err(InvalidTransactionHash::WrongLength(len));
        }
        if !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(InvalidTransactionHash::NotHex);
        }
        Ok(Self(trimmed.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TransactionHash {
    type Err = InvalidTransactionHash;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for TransactionHash {
    type Error = InvalidTransactionHash;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<TransactionHash> for String {
    fn from(value: TransactionHash) -> Self {
        value.0
    }
}

impl fmt::Display for TransactionHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "362e1f1671b3516bfb177a5260ec9c8e24ee6c8fc8e1a1271c8c91db02c9ead7";

    #[test]
    fn accepts_valid_lowercase_hash() {
        let hash = TransactionHash::parse(VALID).expect("valid hash");
        assert_eq!(hash.as_str(), VALID);
    }

    #[test]
    fn normalizes_case_and_whitespace() {
        let input = format!("  {}\n", VALID.to_ascii_uppercase());
        let hash = TransactionHash::parse(&input).expect("valid hash");
        assert_eq!(hash.as_str(), VALID);
    }

    #[test]
    fn rejects_empty_input() {
        assert_eq!(
            TransactionHash::parse("   "),
            Err(InvalidTransactionHash::Empty)
        );
    }

    #[test]
    fn rejects_wrong_length() {
        assert_eq!(
            TransactionHash::parse("abc123"),
            Err(InvalidTransactionHash::WrongLength(6))
        );
        let too_long = format!("{VALID}00");
        assert_eq!(
            TransactionHash::parse(&too_long),
            Err(InvalidTransactionHash::WrongLength(66))
        );
    }

    #[test]
    fn rejects_non_hex_characters() {
        let bad = format!("{}zz", &VALID[..62]);
        assert_eq!(
            TransactionHash::parse(&bad),
            Err(InvalidTransactionHash::NotHex)
        );
    }

    #[test]
    fn rejects_multibyte_input_with_correct_byte_length() {
        // 32 two-byte characters = 64 bytes but only 32 chars.
        let input = "é".repeat(32);
        assert_eq!(
            TransactionHash::parse(&input),
            Err(InvalidTransactionHash::WrongLength(32))
        );
    }

    #[test]
    fn serde_round_trip_validates() {
        let json = format!("\"{VALID}\"");
        let hash: TransactionHash = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&hash).expect("serialize"), json);
        assert!(serde_json::from_str::<TransactionHash>("\"nope\"").is_err());
    }
}
