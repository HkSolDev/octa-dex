use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};
use crate::state::*;
use crate::constants::*;

#[derive(Accounts)]
#[instruction(oracle_feed: Pubkey, base_price: u64, precision_step: u64)]
pub struct InitializePool<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + FlashPool::INIT_SPACE,
        seeds = [SEED_FLASH_POOL, oracle_feed.key().as_ref()],
        bump
    )]
    pub flash_pool: Account<'info, FlashPool>,

    #[account(
        init,
        payer = payer,
        token::mint = mint,
        token::authority = flash_pool,
        seeds = [SEED_VAULT, flash_pool.key().as_ref()],
        bump
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    pub mint: InterfaceAccount<'info, Mint>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn initialize_pool_handler(ctx: Context<InitializePool>, oracle_feed: Pubkey, base_price: u64, precision_step: u64) -> Result<()> {
    let flash_pool = &mut ctx.accounts.flash_pool;
    flash_pool.oracle_feed = oracle_feed;
    flash_pool.base_price = base_price;
    flash_pool.precision_step = precision_step;
    flash_pool.entry_fee = ENTRY_FEE;
    flash_pool.total_participants = 0;
    flash_pool.total_pool_amount = 0;
    flash_pool.histogram_buckets = [0; 100];
    flash_pool.outcome = 0;
    flash_pool.median_error = 0;
    flash_pool.bump = ctx.bumps.flash_pool;

    msg!("FlashPool initialized for oracle: {} at base: {} step: {}", oracle_feed, base_price, precision_step);
    Ok(())
}
