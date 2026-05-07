use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::error::ErrorCode;

// ─── Constants ────────────────────────────────────────────────────────────────

/// The official Pyth Solana Receiver program ID.
/// This is the program that OWNS all PriceUpdateV2 accounts.
/// We check this to ensure nobody passes a fake oracle account.
pub const PYTH_RECEIVER_PROGRAM_ID: Pubkey = pubkey!("rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ");

/// The SOL/USD Feed ID (as a 32-byte array derived from the hex string).
/// Hex: 0xef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d
/// Used to verify the price account contains the right asset, not BTC or ETH.
pub const SOL_USD_FEED_ID: [u8; 32] = [
    0xef, 0x0d, 0x8b, 0x6f, 0xda, 0x2c, 0xeb, 0xa4,
    0x1d, 0xa1, 0x5d, 0x40, 0x95, 0xd1, 0xda, 0x39,
    0x2a, 0x0d, 0x2f, 0x8e, 0xd0, 0xc6, 0xc7, 0xbc,
    0x0f, 0x4c, 0xfa, 0xc8, 0xc2, 0x80, 0xb5, 0x6d,
];

/// Maximum age of the price feed in seconds before we reject it as stale.
pub const MAX_PRICE_AGE_SECONDS: i64 = 60;

// ─── PriceUpdateV2 Raw Byte Offsets ──────────────────────────────────────────
//
// We manually decode the PriceUpdateV2 account without importing the Pyth SDK.
// This avoids the borsh 0.10 vs 1.0 version conflict that breaks Anchor 1.0.
//
// The layout is derived from the pyth-solana-receiver-sdk source:
//   pub const LEN: usize = 8 + 32 + 2 + 32 + 8 + 8 + 4 + 8 + 8 + 8 + 8 + 8;
//
// Byte Map:
//   [0  .. 8 ]  = Anchor discriminator (8 bytes)
//   [8  .. 40]  = write_authority: Pubkey (32 bytes)
//   [40 .. 42]  = verification_level: 1 byte tag + 1 byte num_signatures (2 bytes total)
//   [42 .. 74]  = feed_id: [u8; 32]
//   [74 .. 82]  = price: i64
//   [82 .. 90]  = conf: u64
//   [90 .. 94]  = exponent: i32
//   [94 .. 102] = publish_time: i64
//   [102.. 110] = prev_publish_time: i64
//   [110.. 118] = ema_price: i64
//   [118.. 126] = ema_conf: u64
//   [126.. 134] = posted_slot: u64
const OFFSET_FEED_ID:      usize = 42;
const OFFSET_PRICE:        usize = 74;
const OFFSET_EXPONENT:     usize = 90;
const OFFSET_PUBLISH_TIME: usize = 94;

// ─── Accounts ─────────────────────────────────────────────────────────────────

#[derive(Accounts)]
pub struct ResolveMarket<'info> {
    #[account(
        mut,
        seeds = [SEED_FLASH_POOL, flash_pool.oracle_feed.as_ref()],
        bump = flash_pool.bump,
    )]
    pub flash_pool: Account<'info, FlashPool>,

    /// The Pyth PriceUpdateV2 account for this pool's oracle feed.
    ///
    /// # Security
    /// This is `UncheckedAccount` because we need to manually:
    ///   1. Verify the account's owner is the Pyth Receiver Program.
    ///   2. Verify the feed_id bytes match the expected SOL/USD feed.
    ///   3. Check the publish_time for staleness.
    /// All three checks happen inside the handler below.
    ///
    /// # For Tests (LiteSVM)
    /// In your test, use `svm.set_account()` to inject a fake account at this address.
    /// The fake data must be laid out according to the byte map above.
    /// Only the feed_id, price, exponent, and publish_time bytes matter.
    ///
    /// # For Production (MagicBlock ER)
    /// Pass the PDA derived from:
    ///   seeds = [b"price feed", b"pyth-lazer", &feed_id]
    ///   program = PriCems5tHihc6UDXDjzjeawomAwBduWMGAi8ZUjppd
    /// The MagicBlock chain-pusher updates this account every ~50ms.
    ///
    /// CHECK: Manually verified below (owner, feed_id, staleness)
    pub price_update: UncheckedAccount<'info>,

    /// The authority that triggers resolution. Prevents anyone from
    /// randomly resolving the market before the round ends.
    #[account(mut)]
    pub payer: Signer<'info>,
}

// ─── Handler ──────────────────────────────────────────────────────────────────

pub fn resolve_market_handler(ctx: Context<ResolveMarket>) -> Result<()> {
    // ── Security Check 1: Owner verification ──────────────────────────────────
    // Verify this account is owned by the Pyth Receiver Program.
    // If someone passes a fake account they control, this check will fail.
    require_keys_eq!(
        *ctx.accounts.price_update.owner,
        PYTH_RECEIVER_PROGRAM_ID,
        ErrorCode::InvalidOracleOwner
    );

    // ── Decode Raw Bytes ──────────────────────────────────────────────────────
    let data = ctx.accounts.price_update.try_borrow_data()?;
    require!(data.len() >= OFFSET_PUBLISH_TIME + 8, ErrorCode::InvalidOracleData);

    // ── Security Check 2: Feed ID verification ────────────────────────────────
    // Verify the feed_id bytes match SOL/USD.
    // This prevents an attacker from passing a valid BTC/USD price account
    // when the pool is expecting SOL/USD, which would corrupt the outcome.
    let feed_id: [u8; 32] = data[OFFSET_FEED_ID..OFFSET_FEED_ID + 32]
        .try_into()
        .map_err(|_| ErrorCode::InvalidOracleData)?;
    require!(feed_id == SOL_USD_FEED_ID, ErrorCode::MismatchedFeedId);

    // ── Decode price & exponent ───────────────────────────────────────────────
    // `price` is in units of 10^exponent. For SOL/USD, exponent is typically -8.
    // e.g., price = 15_000_000_000 with exponent = -8 means $150.00.
    let raw_price = i64::from_le_bytes(
        data[OFFSET_PRICE..OFFSET_PRICE + 8].try_into().map_err(|_| ErrorCode::InvalidOracleData)?
    );
    let exponent = i32::from_le_bytes(
        data[OFFSET_EXPONENT..OFFSET_EXPONENT + 4].try_into().map_err(|_| ErrorCode::InvalidOracleData)?
    );

    // ── Security Check 3: Staleness check ────────────────────────────────────
    // Reject prices older than MAX_PRICE_AGE_SECONDS.
    // This prevents replaying an old price from a previous slot to manipulate the outcome.
    let publish_time = i64::from_le_bytes(
        data[OFFSET_PUBLISH_TIME..OFFSET_PUBLISH_TIME + 8].try_into().map_err(|_| ErrorCode::InvalidOracleData)?
    );
    let clock = Clock::get()?;
    require!(
        clock.unix_timestamp - publish_time <= MAX_PRICE_AGE_SECONDS,
        ErrorCode::StaleOracle
    );

    // We must drop the borrow before mutably borrowing flash_pool below.
    drop(data);

    // ── Normalize price to our precision ─────────────────────────────────────
    // Our histogram uses base_price with 2 implied decimal places (e.g., 95000 = $950.00).
    // Pyth's exponent for SOL/USD is -8, so we scale:
    //   oracle_price = raw_price * 10^exponent, normalized to our 2-decimal format.
    //
    // Formula: price_in_our_units = raw_price / 10^(abs(exponent) - 2)
    // Example: raw_price=15_000_000_000, exponent=-8 → 15_000_000_000 / 10^6 = 15_000 = $150.00
    let scale = 10_u64.pow((exponent.unsigned_abs()).saturating_sub(2));
    let oracle_price = if scale > 0 {
        (raw_price.unsigned_abs()) / scale
    } else {
        raw_price.unsigned_abs()
    };

    let flash_pool = &mut ctx.accounts.flash_pool;

    // Save the oracle price as the round outcome
    flash_pool.outcome = oracle_price;

    // ── Find the winning bucket ───────────────────────────────────────────────
    let winning_bucket_index = if oracle_price < flash_pool.base_price {
        0usize
    } else {
        let diff = oracle_price - flash_pool.base_price;
        let idx = (diff / flash_pool.precision_step) as usize;
        // Cap to last bucket if price went above our range
        if idx >= HISTOGRAM_BUCKETS { HISTOGRAM_BUCKETS - 1 } else { idx }
    };

    // ── Median Error Calculation ──────────────────────────────────────────────
    // Walk outward from the winning bucket until 50th percentile of
    // participants is covered. This "distance" is the median_error.
    let total_participants = flash_pool.total_participants;

    if total_participants > 0 {
        let target_count = (total_participants / 2).max(1);
        let mut accumulated: u32 = 0;
        let mut median_error: u64 = 0;

        for distance in 0..HISTOGRAM_BUCKETS {
            // Accumulate the bucket at `distance` to the left
            if distance > 0 {
                if let Some(left_idx) = winning_bucket_index.checked_sub(distance) {
                    accumulated = accumulated.saturating_add(flash_pool.histogram_buckets[left_idx]);
                }
            } else {
                // distance == 0: just the winning bucket itself
                accumulated = accumulated.saturating_add(flash_pool.histogram_buckets[winning_bucket_index]);
            }

            // Accumulate the bucket at `distance` to the right
            if distance > 0 {
                let right_idx = winning_bucket_index + distance;
                if right_idx < HISTOGRAM_BUCKETS {
                    accumulated = accumulated.saturating_add(flash_pool.histogram_buckets[right_idx]);
                }
            }

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
        "Market resolved — raw_price: {}, exponent: {}, outcome: {}, winning bucket: {}, median_error: {}",
        raw_price,
        exponent,
        oracle_price,
        winning_bucket_index,
        flash_pool.median_error
    );

    Ok(())
}
