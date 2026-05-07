use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct FlashPool {
    pub oracle_feed: Pubkey,
    pub base_price: u64,
    pub precision_step: u64,
    pub entry_fee: u64,
    pub total_participants: u32,
    pub total_pool_amount: u64,
    pub histogram_buckets: [u32; 100],
    pub outcome: u64,
    pub median_error: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct UserPrediction {
    pub user: Pubkey,
    pub pool: Pubkey,
    pub predicted_bucket_index: u8,
    pub bump: u8,
}

