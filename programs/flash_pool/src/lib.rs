pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("7fMKkQ9dbkMf1FGTv4vZ1m8bgBX1PKehVS6gkDn84Trv");

#[program]
pub mod flash_pool {
    use super::*;

    /// 1. Sets up the market, including the vault and histogram state.
    pub fn initialize_pool(ctx: Context<InitializePool>, oracle_feed: Pubkey, base_price: u64, precision_step: u64) -> Result<()> {
        initialize::initialize_pool_handler(ctx, oracle_feed, base_price, precision_step)
    }

    /// 2. Records a user's prediction and collects the entry fee.
    pub fn place_prediction(ctx: Context<PlacePrediction>, prediction_value: u64) -> Result<()> {
        place_prediction::place_prediction_handler(ctx, prediction_value)
    }

    /// 3. Resolves the market round based on an oracle price and calculates the median error.
    pub fn resolve_market(ctx: Context<ResolveMarket>, oracle_price: u64) -> Result<()> {
        resolve_market::resolve_market_handler(ctx, oracle_price)
    }

    /// 4. Payouts winners and reclaims rent from prediction PDAs.
    pub fn claim_reward(ctx: Context<ClaimReward>) -> Result<()> {
        claim_reward::claim_reward_handler(ctx)
    }
}
