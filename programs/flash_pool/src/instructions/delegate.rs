use anchor_lang::prelude::*;
use crate::constants::SEED_FLASH_POOL;

pub const DELEGATION_PROGRAM_ID: Pubkey =
    pubkey!("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");

/// Minimal accounts struct for the Opaque CPI Proxy instruction.
/// All MagicBlock-internal accounts arrive via `remaining_accounts`
/// so this struct never needs updating when MagicBlock changes their layout.
#[derive(Accounts)]
pub struct DelegateProxy<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: only its key is used to re-derive the FlashPool PDA seeds
    pub oracle_feed: UncheckedAccount<'info>,

    // UncheckedAccount prevents Anchor auto-serializing after ownership
    // transfers to the Delegation Program (→ ExternalAccountDataModified crash).
    // Seeds + bump constraint still enforces correct PDA derivation.
    /// CHECK: PDA verified by seeds constraint. UncheckedAccount used intentionally
    /// to prevent Anchor re-serializing FlashPool state after the Delegation Program
    /// takes ownership of this account during the proxy CPI.
    #[account(
        mut,
        seeds = [SEED_FLASH_POOL, oracle_feed.key().as_ref()],
        bump
    )]
    pub flash_pool: UncheckedAccount<'info>,

    /// CHECK: MagicBlock Delegation Program — verified by address constraint
    #[account(address = DELEGATION_PROGRAM_ID)]
    pub delegation_program: UncheckedAccount<'info>,
}
