use anchor_lang::prelude::*;
use anchor_spl::token_interface::{TokenAccount, TokenInterface, TransferChecked, transfer_checked, Mint};
use crate::state::*;
use crate::constants::*;
use crate::error::ErrorCode;

#[derive(Accounts)]
pub struct PlacePrediction<'info> {
    #[account(
        mut,
        seeds = [SEED_FLASH_POOL, flash_pool.oracle_feed.as_ref()],
        bump = flash_pool.bump,
    )]
    pub flash_pool: Account<'info, FlashPool>,

    #[account(
        init,
        payer = user,
        space = 8 + UserPrediction::INIT_SPACE,
        seeds = [SEED_USER_PREDICTION, flash_pool.key().as_ref(), user.key().as_ref()],
        bump
    )]
    pub user_prediction: Account<'info, UserPrediction>,

    #[account(
        mut,
        seeds = [SEED_VAULT, flash_pool.key().as_ref()],
        bump
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    /// The USDC mint — required by transfer_checked for decimal safety
    pub mint: InterfaceAccount<'info, Mint>,

    #[account(mut)]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn place_prediction_handler(ctx: Context<PlacePrediction>, prediction_value: u64) -> Result<()> {
    let flash_pool = &mut ctx.accounts.flash_pool;

    // 1. Map user's price guess to a histogram bucket index
    require!(prediction_value >= flash_pool.base_price, ErrorCode::InvalidPrediction);
    let diff = prediction_value - flash_pool.base_price;
    let bucket_index = (diff / flash_pool.precision_step) as usize;
    require!(bucket_index < HISTOGRAM_BUCKETS, ErrorCode::InvalidPrediction);

    // 2. Transfer 1 USDC entry fee from user → vault using transfer_checked
    let decimals = ctx.accounts.mint.decimals;
    let cpi_accounts = TransferChecked {
        from: ctx.accounts.user_token_account.to_account_info(),
        mint: ctx.accounts.mint.to_account_info(),
        to: ctx.accounts.vault.to_account_info(),
        authority: ctx.accounts.user.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(
        ctx.accounts.token_program.key(),
        cpi_accounts,
    );
    transfer_checked(cpi_ctx, flash_pool.entry_fee, decimals)?;

    // 3. Update global pool state
    flash_pool.total_participants = flash_pool
        .total_participants
        .checked_add(1)
        .ok_or(ErrorCode::Overflow)?;
    flash_pool.total_pool_amount = flash_pool
        .total_pool_amount
        .checked_add(flash_pool.entry_fee)
        .ok_or(ErrorCode::Overflow)?;
    flash_pool.histogram_buckets[bucket_index] = flash_pool.histogram_buckets[bucket_index]
        .checked_add(1)
        .ok_or(ErrorCode::Overflow)?;

    // 4. Create the user's prediction receipt PDA
    let user_prediction = &mut ctx.accounts.user_prediction;
    user_prediction.user = ctx.accounts.user.key();
    user_prediction.pool = flash_pool.key();
    user_prediction.predicted_bucket_index = bucket_index as u8;
    user_prediction.bump = ctx.bumps.user_prediction;

    msg!("Prediction placed in bucket {} for pool {}", bucket_index, flash_pool.key());
    Ok(())
}
