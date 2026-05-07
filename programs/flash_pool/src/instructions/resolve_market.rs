use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;

#[derive(Accounts)]
pub struct ResolveMarket<'info> {
    #[account(
        mut,
        seeds = [SEED_FLASH_POOL, flash_pool.oracle_feed.as_ref()],
        bump = flash_pool.bump,
    )]
    pub flash_pool: Account<'info, FlashPool>,

    #[account(mut)]
    pub payer: Signer<'info>,
}

/// For the demo we accept `oracle_price` as an argument.
/// When upgrading to live Pyth Lazer, replace this argument with
/// the ephemeral oracle AccountInfo and deserialize it here.
pub fn resolve_market_handler(ctx: Context<ResolveMarket>, oracle_price: u64) -> Result<()> {
    let flash_pool = &mut ctx.accounts.flash_pool;

    // Save the oracle price as the round outcome
    flash_pool.outcome = oracle_price;

    // Find which bucket the oracle price falls into
    let winning_bucket_index = if oracle_price < flash_pool.base_price {
        0usize
    } else {
        let diff = oracle_price - flash_pool.base_price;
        let idx = (diff / flash_pool.precision_step) as usize;
        // Cap to last bucket if price went above our range
        if idx >= HISTOGRAM_BUCKETS { HISTOGRAM_BUCKETS - 1 } else { idx }
    };

    // Walk outward from winning bucket until 50th percentile is covered (median error)
    let total_participants = flash_pool.total_participants;

    if total_participants > 0 {
        let target_count = (total_participants / 2).max(1); // at least 1
        let mut accumulated: u32 = 0;
        let mut median_error: u64 = 0;

            // We walk outward (distance 0, 1, 2...) until we accumulate 50% of the votes.
            // Accumulate the bucket at `distance` to the left.
            if distance > 0 {
                if let Some(left_idx) = winning_bucket_index.checked_sub(distance) {
                    accumulated = accumulated.saturating_add(flash_pool.histogram_buckets[left_idx]);
                }
            } else {
                // distance == 0: just the winning bucket itself.
                accumulated = accumulated.saturating_add(flash_pool.histogram_buckets[winning_bucket_index]);
            }

            // Accumulate the bucket at `distance` to the right (skip 0, already counted above).
            if distance > 0 {
                let right_idx = winning_bucket_index + distance;
                if right_idx < HISTOGRAM_BUCKETS {
                    accumulated = accumulated.saturating_add(flash_pool.histogram_buckets[right_idx]);
                }
            }

            // The 'median_error' is the distance required to reach the target_count.
            median_error = distance as u64;

            if accumulated >= target_count {
                break;
            }
        }

        flash_pool.median_error = median_error;
    } else {
        flash_pool.median_error = 0;
    }

    msg!(
        "Market resolved — outcome: {}, winning bucket: {}, median_error: {}",
        oracle_price,
        winning_bucket_index,
        flash_pool.median_error
    );

    Ok(())
}
