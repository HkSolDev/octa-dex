
/// # 10,000 User Stress Test
///
/// ## The Core Trick — `svm.set_account()`
///
/// Instead of doing 10,000 real SPL Token transfers, we use LiteSVM's superpower:
/// injecting arbitrary account states directly into SVM memory. We use the
/// Token-2022 `Pack` trait to correctly serialize token accounts, bypassing the
/// token program's `mint_to` instruction entirely.
///
/// ## Why this simulates MagicBlock Ephemeral Rollups
///
/// MagicBlock ERs strip away L1 consensus. LiteSVM strips away networking and
/// block production. Both execute the SVM directly in-memory. Running 10,000
/// transactions in LiteSVM in under a second is the best local approximation
/// of what happens inside a MagicBlock ER at high throughput.

use {
    anchor_lang::{
        prelude::Pubkey,
        system_program,
        solana_program::instruction::Instruction,
        InstructionData, ToAccountMetas,
        AccountDeserialize,
    },
    spl_token_2022_interface::state::{Account as SplTokenAccount, AccountState},
    solana_program_pack::Pack,
    solana_program_option::COption,
    litesvm::LiteSVM,
    solana_account::Account,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_keypair::Keypair,
    solana_transaction::versioned::VersionedTransaction,
    flash_pool::state::FlashPool,
};

/// Helper: inject a properly-packed Token-2022 account into SVM memory.
/// Uses spl_token_2022's own `Pack` trait so we get the exact byte layout
/// the token program expects — including the account_type discriminator byte.
fn inject_token_account(
    svm: &mut LiteSVM,
    address: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    amount: u64,
) {
    let token_account = SplTokenAccount {
        mint,
        owner,
        amount,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };

    let mut packed = vec![0u8; SplTokenAccount::LEN];
    SplTokenAccount::pack(token_account, &mut packed).unwrap();

    let rent_lamports = svm.minimum_balance_for_rent_exemption(SplTokenAccount::LEN);
    svm.set_account(
        address,
        Account {
            lamports: rent_lamports,
            data: packed,
            owner: anchor_spl::token_2022::ID,
            executable: false,
            rent_epoch: 0,
        },
    ).expect("Failed to inject token account");
}

/// Helper: build + send InitializePool.
fn initialize_pool(
    svm: &mut LiteSVM,
    program_id: Pubkey,
    payer: &Keypair,
    oracle_feed: Pubkey,
    mint: Pubkey,
) -> (Pubkey, Pubkey) {
    let (flash_pool_pda, _) = Pubkey::find_program_address(
        &[b"flash_pool", oracle_feed.as_ref()], &program_id,
    );
    let (vault_pda, _) = Pubkey::find_program_address(
        &[b"vault", flash_pool_pda.as_ref()], &program_id,
    );

    let ix = Instruction::new_with_bytes(
        program_id,
        &flash_pool::instruction::InitializePool {
            oracle_feed,
            base_price: 95_000, // base bucket 0 = $950.00
            precision_step: 10, // $0.10 per bucket step
        }.data(),
        flash_pool::accounts::InitializePool {
            flash_pool: flash_pool_pda,
            vault: vault_pda,
            mint,
            payer: payer.pubkey(),
            token_program: anchor_spl::token_2022::ID,
            system_program: system_program::ID,
            rent: anchor_lang::prelude::rent::ID,
        }.to_account_metas(None),
    );

    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &svm.latest_blockhash());
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[payer]).unwrap();
    svm.send_transaction(tx).expect("InitializePool failed");

    (flash_pool_pda, vault_pda)
}

#[test]
fn test_stress_10k_predictions() {
    // ─── Setup ──────────────────────────────────────────────────────────────
    let program_id = flash_pool::id();
    let payer = Keypair::new();
    let mint_keypair = Keypair::new();
    let mint_pubkey = mint_keypair.pubkey();
    let oracle_feed = Pubkey::new_unique();

    let mut svm = LiteSVM::new();
    let bytes = include_bytes!("../../../target/deploy/flash_pool.so");
    svm.add_program(program_id, bytes).unwrap();
    svm.airdrop(&payer.pubkey(), 100_000_000_000).unwrap(); // 100 SOL

    // ─── Create a real Token-2022 Mint ──────────────────────────────────────
    let rent = svm.minimum_balance_for_rent_exemption(82);
    let create_mint_ix = anchor_lang::solana_program::system_instruction::create_account(
        &payer.pubkey(), &mint_pubkey, rent, 82, &anchor_spl::token_2022::ID,
    );
    let init_mint_ix = anchor_spl::token_2022::spl_token_2022::instruction::initialize_mint(
        &anchor_spl::token_2022::ID, &mint_pubkey, &payer.pubkey(), None, 6,
    ).unwrap();
    let msg = Message::new_with_blockhash(&[create_mint_ix, init_mint_ix], Some(&payer.pubkey()), &svm.latest_blockhash());
    svm.send_transaction(VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[&payer, &mint_keypair]).unwrap()).expect("Mint init failed");

    // ─── Initialize FlashPool ───────────────────────────────────────────────
    let (flash_pool_pda, vault_pda) = initialize_pool(&mut svm, program_id, &payer, oracle_feed, mint_pubkey);

    // ─── Inject real USDC into the vault so CPI token transfers succeed ─────
    // The vault already exists (initialized by the pool). We top it up with
    // enough USDC to represent the full stress pool.
    const NUM_USERS: usize = 10_000;
    const BASE_PRICE: u64 = 95_000;
    const PRECISION_STEP: u64 = 10;
    const ENTRY_FEE: u64 = 1_000_000; // 1 USDC (6 decimals)

    // Pre-load the vault with enough USDC so the on-chain CPI
    // transfer_checked (user → vault) doesn't need real minting.
    // We inject the user's ATA with ENTRY_FEE. For the vault we rely on
    // the transfer_checked CPI to ADD to the balance normally.
    // But since we're injecting user ATAs with enough USDC,
    // the on-chain CPI will deduct from user ATAs and credit the vault naturally.

    println!("\n🚀 Starting stress test — {} predictions...", NUM_USERS);
    let start = std::time::Instant::now();

    for i in 0..NUM_USERS {
        let user = Keypair::new();
        let user_pubkey = user.pubkey();

        // Airdrop SOL for rent (UserPrediction PDA creation) + tx fees
        svm.airdrop(&user_pubkey, 5_000_000).unwrap();

        // ── KEY TRICK: Inject fake USDC token account via svm.set_account() ──
        // This replaces 10,000 real `mint_to` transactions with direct memory
        // injection. The SVM validates balance during the on-chain CPI,
        // so the account must be correctly packed with the Token-2022 layout.
        let user_ata = Pubkey::new_unique();
        inject_token_account(&mut svm, user_ata, mint_pubkey, user_pubkey, ENTRY_FEE * 2);

        // Derive UserPrediction PDA for this user
        let (user_prediction_pda, _) = Pubkey::find_program_address(
            &[b"user_prediction", flash_pool_pda.as_ref(), user_pubkey.as_ref()],
            &program_id,
        );

        // 4. Place Prediction
        // We spread the predictions evenly across all 100 buckets to test the math.
        // Each bucket 'i % 100' gets an equal share of the 10,000 users.
        let bucket: u64 = (i % 100) as u64;
        let prediction_value = BASE_PRICE + (bucket * PRECISION_STEP);

        let ix = Instruction::new_with_bytes(
            program_id,
            &flash_pool::instruction::PlacePrediction { prediction_value }.data(),
            flash_pool::accounts::PlacePrediction {
                flash_pool: flash_pool_pda,
                user_prediction: user_prediction_pda,
                vault: vault_pda,
                mint: mint_pubkey,
                user_token_account: user_ata,
                user: user_pubkey,
                token_program: anchor_spl::token_2022::ID,
                system_program: system_program::ID,
            }.to_account_metas(None),
        );

        // Sign with the unique user keypair for this transaction.
        let msg = Message::new_with_blockhash(&[ix], Some(&user_pubkey), &svm.latest_blockhash());
        let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[&user]).unwrap();

        // Send directly to the in-memory SVM. No networking overhead.
        svm.send_transaction(tx)
            .unwrap_or_else(|e| panic!("tx {} (bucket {}) failed: {:?}", i, bucket, e));
    }

    let duration = start.elapsed();
    let tps = NUM_USERS as f64 / duration.as_secs_f64();
    println!("✅ {} predictions processed in {:.2?} ({:.0} tx/s)", NUM_USERS, duration, tps);

    // ─── State Verification ─────────────────────────────────────────────────
    let pool_acc = svm.get_account(&flash_pool_pda).expect("FlashPool not found");
    // Note: AccountDeserialize::try_deserialize handles the 8-byte discriminator
    let pool = FlashPool::try_deserialize(&mut &pool_acc.data[..])
        .expect("Failed to deserialize FlashPool");

    // 1. Total participant count
    assert_eq!(
        pool.total_participants, NUM_USERS as u32,
        "Participants: expected {}, got {}", NUM_USERS, pool.total_participants
    );

    // 2. Total USDC collected
    assert_eq!(
        pool.total_pool_amount, NUM_USERS as u64 * ENTRY_FEE,
        "Pool amount: expected {}, got {}", NUM_USERS as u64 * ENTRY_FEE, pool.total_pool_amount
    );

    // 3. Each bucket should have exactly NUM_USERS/100 = 100 users
    let expected_per_bucket: u32 = (NUM_USERS / 100) as u32;
    for (idx, &count) in pool.histogram_buckets.iter().enumerate() {
        assert_eq!(
            count, expected_per_bucket,
            "Bucket {} has {} users, expected {}", idx, count, expected_per_bucket
        );
    }

    println!("✅ State verified:");
    println!("   • Total participants : {}", pool.total_participants);
    println!("   • Users per bucket   : {} (across 100 buckets)", expected_per_bucket);
    println!("   • Total USDC in vault: {} USDC", pool.total_pool_amount / 1_000_000);
    println!("   • Throughput         : {:.0} tx/s (MagicBlock ER simulation)", tps);
}
