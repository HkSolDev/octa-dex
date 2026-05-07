use anchor_lang::prelude::*;

#[constant]
pub const SEED_FLASH_POOL: &[u8] = b"flash_pool";
#[constant]
pub const SEED_USER_PREDICTION: &[u8] = b"user_prediction";
#[constant]
pub const SEED_VAULT: &[u8] = b"vault";

pub const ENTRY_FEE: u64 = 1_000_000; // 1 USDC (6 decimals)
pub const HISTOGRAM_BUCKETS: usize = 100;
