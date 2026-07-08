use std::{
    error::Error,
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
};

use alloy::primitives::{B256, U256};
use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::BytesMut;
use fmt::Display;
use serde::{Deserialize, Serialize};
use tokio_postgres::types::{FromSql, IsNull, ToSql, Type};
use tracing::warn;

use crate::shared::from_param_u256;

#[derive(Debug, Copy, Clone, Serialize, Deserialize, Eq)]
pub struct BlockHash(B256);

impl BlockHash {
    pub fn new(block_hash: B256) -> Self {
        BlockHash(block_hash)
    }
}

impl Hash for BlockHash {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl PartialEq for BlockHash {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<'a> FromSql<'a> for BlockHash {
    fn from_sql(_ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        parse_block_hash_from_sql_bytes(raw).map_err(Into::into)
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::BYTEA
    }
}

impl ToSql for BlockHash {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        out.extend_from_slice(self.0.as_slice());
        Ok(IsNull::No)
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::BYTEA
    }

    tokio_postgres::types::to_sql_checked!();
}

#[derive(Debug)]
pub struct ParseBlockHashError(String);

impl Display for ParseBlockHashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid block hash: {}", self.0)
    }
}

impl Error for ParseBlockHashError {}

impl FromStr for BlockHash {
    type Err = ParseBlockHashError;

    fn from_str(param: &str) -> Result<Self, Self::Err> {
        from_param_u256(param)
            .map(|hash| BlockHash(hash.into()))
            .map_err(|e| ParseBlockHashError(e.to_string()))
    }
}

impl From<BlockHash> for B256 {
    fn from(block_hash: BlockHash) -> Self {
        block_hash.0
    }
}

impl From<BlockHash> for U256 {
    fn from(block_hash: BlockHash) -> Self {
        block_hash.0.into()
    }
}

impl From<U256> for BlockHash {
    fn from(block_hash: U256) -> Self {
        BlockHash(B256::from(block_hash))
    }
}

fn parse_block_hash_from_sql_bytes(raw: &[u8]) -> Result<BlockHash, String> {
    if raw.len() == 32 {
        return Ok(BlockHash(B256::from_slice(raw)));
    }

    let raw_text = std::str::from_utf8(raw)
        .map_err(|_| format!("Block hash BYTEA length was {}, and legacy textual decode failed (bytes are not valid UTF-8). Please run a migration to rewrite block_hash values as raw 32-byte BYTEA.", raw.len()))?;
    let raw_text = raw_text.trim();

    let maybe_hex = raw_text.strip_prefix("0x").unwrap_or(raw_text);
    if maybe_hex.len() == 64 {
        match B256::from_str(raw_text) {
            Ok(hash) => {
                warn!(
                    "Decoded block hash from legacy textual hex BYTEA value; migrate rows to raw 32-byte format"
                );
                return Ok(BlockHash(hash));
            }
            Err(error) => {
                return Err(format!(
                    "Block hash BYTEA length was {}, looked like legacy hex text but parsing failed: {}. Please run a migration to rewrite block_hash values as raw 32-byte BYTEA.",
                    raw.len(),
                    error
                ));
            }
        }
    }

    match STANDARD.decode(raw_text) {
        Ok(decoded) if decoded.len() == 32 => {
            warn!(
                "Decoded block hash from legacy textual base64 BYTEA value; migrate rows to raw 32-byte format"
            );
            Ok(BlockHash(B256::from_slice(&decoded)))
        }
        Ok(decoded) => Err(format!(
            "Block hash BYTEA length was {}, and legacy base64 decoded to {} bytes (expected 32). Please run a migration to rewrite block_hash values as raw 32-byte BYTEA.",
            raw.len(),
            decoded.len()
        )),
        Err(error) => Err(format!(
            "Block hash BYTEA length was {}, and legacy textual decode failed (expected 64-char hex, 0x-prefixed hex, or base64): {}. Please run a migration to rewrite block_hash values as raw 32-byte BYTEA.",
            raw.len(),
            error
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_hash_bytea_round_trips_as_raw_hash_bytes() {
        let alloy_hash = B256::repeat_byte(0xab);
        let block_hash = BlockHash::new(alloy_hash);
        let mut out = BytesMut::new();

        let is_null = block_hash.to_sql(&Type::BYTEA, &mut out).unwrap();

        assert!(matches!(is_null, IsNull::No));
        assert_eq!(out.len(), 32);
        assert_eq!(out.as_ref(), alloy_hash.as_slice());

        let decoded = BlockHash::from_sql(&Type::BYTEA, out.as_ref()).unwrap();

        assert_eq!(decoded, block_hash);
    }

    #[test]
    fn block_hash_bytea_rejects_invalid_length() {
        let err = BlockHash::from_sql(&Type::BYTEA, &[0xab; 31]).unwrap_err();

        assert_eq!(
            "Block hash BYTEA length was 31, and legacy textual decode failed (bytes are not valid UTF-8). Please run a migration to rewrite block_hash values as raw 32-byte BYTEA.",
            err.to_string()
        );
    }

    #[test]
    fn block_hash_bytea_accepts_legacy_hex_text() {
        let alloy_hash = B256::repeat_byte(0xcd);
        let legacy_text = alloy_hash.to_string();

        let decoded = BlockHash::from_sql(&Type::BYTEA, legacy_text.as_bytes()).unwrap();

        assert_eq!(decoded, BlockHash::new(alloy_hash));
    }

    #[test]
    fn block_hash_bytea_accepts_legacy_base64_text() {
        let alloy_hash = B256::repeat_byte(0xef);
        let legacy_text = STANDARD.encode(alloy_hash.as_slice());

        let decoded = BlockHash::from_sql(&Type::BYTEA, legacy_text.as_bytes()).unwrap();

        assert_eq!(decoded, BlockHash::new(alloy_hash));
    }
}
