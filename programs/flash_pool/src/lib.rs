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

    pub fn initialize_pool(ctx: Context<InitializePool>, oracle_feed: Pubkey, base_price: u64, precision_step: u64) -> Result<()> {
        initialize::initialize_pool_handler(ctx, oracle_feed, base_price, precision_step)
    }

    pub fn place_prediction(ctx: Context<PlacePrediction>, prediction_value: u64) -> Result<()> {
        place_prediction::place_prediction_handler(ctx, prediction_value)
    }

    pub fn resolve_market(ctx: Context<ResolveMarket>, oracle_price: u64) -> Result<()> {
        resolve_market::resolve_market_handler(ctx, oracle_price)
    }

    pub fn claim_reward(ctx: Context<ClaimReward>) -> Result<()> {
        claim_reward::claim_reward_handler(ctx)
    }
}
