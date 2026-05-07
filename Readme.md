# Octa-Dex: FlashPool Prediction Market

Octa-Dex is a high-performance, accuracy-based prediction market built on Solana using the Anchor framework. It leverages a unique **Histogram-based bucketing system** to enable $O(1)$ market resolution and payout calculations, making it ideal for high-throughput environments like MagicBlock's Ephemeral Rollups.

## 🚀 Key Features

- **O(1) Resolution**: Market resolution complexity is independent of the number of participants.
- **Accuracy-Based Payouts**: Rewards are distributed based on prediction proximity to the actual outcome (Median Error).
- **Token-2022 Integration**: Native support for modern SPL Token features.
- **Optimized Memory**: Uses heap-allocated (boxed) accounts to bypass Solana's 4KB stack limit.
- **High Performance**: Designed to handle 10,000+ predictions in seconds.

---

## 🏗️ Architecture & Design Decisions

### 1. Histogram-Based Bucketing
Instead of storing every individual prediction in a dynamic array (which would lead to $O(N)$ resolution costs and hit account size limits), Octa-Dex uses a fixed-size histogram of 100 buckets.

**Why?**
- **Fixed Account Size**: The `FlashPool` state remains constant regardless of whether 10 or 10,000 users join.
- **Instant Resolution**: Finding the "Median Error" only requires walking through 100 integer counters, not sorting thousands of entries.

**Example:**
If the `base_price` is $950.00 and the `precision_step` is $0.10:
- Bucket 0: $950.00 - $950.09
- Bucket 1: $950.10 - $950.19
- ... and so on.

### 2. Stack Management (Boxing)
Solana has a strict 4KB stack limit. Because our `FlashPool` account contains a 100-element `u32` array and other metadata, loading it into the stack during `ClaimReward` was causing a `Stack frame too large` error.

**The Solution:**
```rust
pub struct ClaimReward<'info> {
    #[account(mut)]
    pub flash_pool: Box<Account<'info, FlashPool>>, // Boxed to move to heap
    ...
}
```
*Why:* `Box` moves the account data from the stack to the heap, ensuring the program doesn't crash during execution.

### 3. Token-2022 & `transfer_checked`
We use `Token-2022` for the pool's vault. This ensures compatibility with modern assets and provides better security via `transfer_checked`.

```rust
transfer_checked(
    CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        TransferChecked {
            from: ctx.accounts.user_token_account.to_account_info(),
            to: ctx.accounts.vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
        },
    ),
    ENTRY_FEE,
    ctx.accounts.mint.decimals,
)?;
```
*Why:* `transfer_checked` verifies the mint and decimals at the CPI level, preventing "false token" attacks.

---

## 🛠️ Instructions

### `initialize_pool`
Sets up the global market state.
- **Parameters**: `oracle_feed`, `base_price`, `precision_step`.
- **Action**: Creates a `FlashPool` PDA and a `Vault` PDA to hold user funds.

### `place_prediction`
Allows a user to enter the pool.
- **Logic**:
  1. Calculates the bucket index: `(predicted_value - base_price) / precision_step`.
  2. Increments the global `histogram_buckets[idx]`.
  3. Deducts the `ENTRY_FEE` from the user and sends it to the vault.
  4. Stores the user's specific prediction in a `UserPrediction` PDA.

### `resolve_market`
Locked the market and determines winners.
- **Median Error Logic**: It walks outward from the winning bucket until it covers 50% of the participants. This "distance" becomes the `median_error`.
- **Result**: Anyone whose prediction is within this distance of the outcome is considered a winner.

### `claim_reward`
Distributes funds to accurate predictors.
- **Math**: `payout = total_pool / winners_count`.
- **Cleanup**: Reclaims the rent from the `UserPrediction` PDA and sends it back to the user (and a fee to the treasury).

---

## 🧪 Testing Suite

We use **LiteSVM** for lightning-fast integration testing. LiteSVM runs the Solana VM directly in memory, bypassing the slow network overhead of `solana-test-validator`.

### 10,000 User Stress Test
We verified the system can handle 10,000 users.
**The "Secret Sauce":** We used `svm.set_account()` to inject 10,000 fake USDC accounts directly into memory. This avoided 10,000 slow `mint_to` transactions, allowing us to simulate a massive load in under 60 seconds.

```rust
// Injected a fake USDC account to speed up the test
svm.set_account(user_ata, Account { ... data: packed_token_data ... }).unwrap();
```

---

## 🚀 Future Roadmap: MagicBlock Integration

The current codebase is "Phase 2 ready."
1. **Pyth Lazer**: The `resolve_market` instruction is designed to be easily swapped with a Pyth Lazer oracle account.
2. **Ephemeral Rollups**: Because our resolution is $O(1)$, we can process thousands of predictions per second on a MagicBlock ER and settle the final state back to Solana L1 seamlessly.

---

