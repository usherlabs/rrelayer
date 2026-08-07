use std::{
    error::Error,
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
};

use alloy::primitives::{B256, U256};
use bytes::BytesMut;
use fmt::Display;
use serde::{Deserialize, Serialize};
use tokio_postgres::types::{FromSql, IsNull, ToSql, Type};

use crate::shared::from_param_u256;

const BLOCK_HASH_BYTE_LENGTH: usize = 32;

#[derive(Debug, thiserror::Error)]
enum BlockHashSqlError {
    #[error("Invalid byte length for block hash: expected {expected}, got {actual}")]
    InvalidByteLength { expected: usize, actual: usize },
}

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
        if raw.len() != BLOCK_HASH_BYTE_LENGTH {
            return Err(BlockHashSqlError::InvalidByteLength {
                expected: BLOCK_HASH_BYTE_LENGTH,
                actual: raw.len(),
            }
            .into());
        }

        let block_hash = B256::from_slice(raw);

        Ok(BlockHash(block_hash))
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
        let err = err.downcast::<BlockHashSqlError>().unwrap();

        assert!(matches!(
            *err,
            BlockHashSqlError::InvalidByteLength { expected: BLOCK_HASH_BYTE_LENGTH, actual: 31 }
        ));
    }
}
