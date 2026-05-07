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

    /// 3. Resolves the market by reading the real-time price from the Pyth oracle account.
    pub fn resolve_market(ctx: Context<ResolveMarket>) -> Result<()> {
        resolve_market::resolve_market_handler(ctx)
    }

    /// 4. Payouts winners and reclaims rent from prediction PDAs.
    pub fn claim_reward(ctx: Context<ClaimReward>) -> Result<()> {
        claim_reward::claim_reward_handler(ctx)
    }

    /// 5. Opaque CPI Proxy — delegates the FlashPool PDA to MagicBlock ER.
    ///
    /// The TS client uses the v0.13 JS SDK to build the exact delegation
    /// instruction, passes raw ix_data + all accounts as remaining_accounts.
    /// This program's only job: forward to Delegation Program + attach PDA sig.
    pub fn delegate_proxy(ctx: Context<DelegateProxy>, ix_data: Vec<u8>) -> Result<()> {
        use anchor_lang::solana_program::{instruction::{AccountMeta, Instruction}, program::invoke_signed};

        let oracle_key     = ctx.accounts.oracle_feed.key();
        let flash_pool_key = ctx.accounts.flash_pool.key();
        let delegation_pgm = ctx.accounts.delegation_program.key();
        let bump           = ctx.bumps.flash_pool;

        let signer_seeds: &[&[&[u8]]] = &[&[
            constants::SEED_FLASH_POOL,
            oracle_key.as_ref(),
            &[bump],
        ]];

        // Build metas + infos from remaining_accounts only.
        // The JS SDK includes the delegation program in the keys list; it will
        // appear in account_infos naturally via this loop.
        let mut accounts      = Vec::with_capacity(ctx.remaining_accounts.len());
        let mut account_infos = Vec::with_capacity(ctx.remaining_accounts.len());

        for account in ctx.remaining_accounts {
            let is_signer = account.is_signer || account.key() == flash_pool_key;
            accounts.push(AccountMeta { pubkey: account.key(), is_signer, is_writable: account.is_writable });
            account_infos.push(account.clone());
        }

        let ix = Instruction { program_id: delegation_pgm, accounts, data: ix_data };

        invoke_signed(&ix, &account_infos, signer_seeds)?;
        msg!("FlashPool {} delegated to ER via proxy CPI", flash_pool_key);
        Ok(())
    }

}
