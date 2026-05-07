use anchor_lang::prelude::*;
use anchor_spl::token_interface::{TokenAccount, TokenInterface, TransferChecked, transfer_checked, Mint};
use crate::state::*;
use crate::constants::*;
use crate::error::ErrorCode;

#[derive(Accounts)]
pub struct ClaimReward<'info> {
    #[account(
        seeds = [SEED_FLASH_POOL, flash_pool.oracle_feed.as_ref()],
        bump = flash_pool.bump,
    )]
    pub flash_pool: Box<Account<'info, FlashPool>>,

    #[account(
        mut,
        close = treasury,
        seeds = [SEED_USER_PREDICTION, flash_pool.key().as_ref(), user.key().as_ref()],
        bump = user_prediction.bump,
        has_one = user,
        constraint = user_prediction.pool == flash_pool.key() @ ErrorCode::Unauthorized,
    )]
    pub user_prediction: Account<'info, UserPrediction>,

    #[account(
        mut,
        seeds = [SEED_VAULT, flash_pool.key().as_ref()],
        bump
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    /// The USDC mint — required by transfer_checked
    pub mint: InterfaceAccount<'info, Mint>,

    /// The winner's token account to receive USDC
    #[account(mut)]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,

    /// CHECK: The user that placed the prediction — verified via has_one on user_prediction
    pub user: UncheckedAccount<'info>,

    /// The protocol treasury that receives the closed PDA's rent
    #[account(mut)]
    pub treasury: SystemAccount<'info>,

    /// The Crank/caller that executes this instruction
    pub payer: Signer<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn claim_reward_handler(ctx: Context<ClaimReward>) -> Result<()> {
    let flash_pool = &ctx.accounts.flash_pool;
    let user_prediction = &ctx.accounts.user_prediction;

    // 1. Verify the market has been resolved (outcome will be non-zero)
    require!(flash_pool.outcome > 0, ErrorCode::Unauthorized);

    // 2. Find the winning bucket from the resolved oracle price
    let winning_bucket_index = if flash_pool.outcome < flash_pool.base_price {
        0usize
    } else {
        let diff = flash_pool.outcome - flash_pool.base_price;
        let idx = (diff / flash_pool.precision_step) as usize;
        if idx >= HISTOGRAM_BUCKETS { HISTOGRAM_BUCKETS - 1 } else { idx }
    };

    // 3. Check this user is a winner — their bucket must be within median_error distance
    let user_bucket = user_prediction.predicted_bucket_index as usize;
    let distance = (user_bucket as i64 - winning_bucket_index as i64).unsigned_abs() as u64;
    require!(distance <= flash_pool.median_error, ErrorCode::Unauthorized);

    // 4. Equal-share payout: total_pool / total_participants
    //    (a weighted-by-accuracy payout is the production upgrade)
    let payout = flash_pool.total_pool_amount
        .checked_div(flash_pool.total_participants as u64)
        .ok_or(ErrorCode::Overflow)?;

    // 5. Transfer USDC from Vault → winner using PDA signer seeds
    let oracle_feed_key = flash_pool.oracle_feed;
    let bump = flash_pool.bump;
    let signer_seeds: &[&[&[u8]]] = &[&[SEED_FLASH_POOL, oracle_feed_key.as_ref(), &[bump]]];

    let decimals = ctx.accounts.mint.decimals;
    let cpi_accounts = TransferChecked {
        from: ctx.accounts.vault.to_account_info(),
        mint: ctx.accounts.mint.to_account_info(),
        to: ctx.accounts.user_token_account.to_account_info(),
        authority: ctx.accounts.flash_pool.to_account_info(),
    };
    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        cpi_accounts,
        signer_seeds,
    );
    transfer_checked(cpi_ctx, payout, decimals)?;

    msg!(
        "Paid {} USDC to user {} (bucket {} within {} of winning bucket {})",
        payout,
        ctx.accounts.user.key(),
        user_bucket,
        flash_pool.median_error,
        winning_bucket_index
    );

    // 6. user_prediction PDA is auto-closed by `close = treasury` — rent goes to treasury

    Ok(())
}
