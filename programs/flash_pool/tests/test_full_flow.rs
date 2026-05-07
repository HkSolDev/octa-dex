
use {
    anchor_lang::{
        prelude::Pubkey,
        system_program,
        solana_program::instruction::Instruction,
        InstructionData, ToAccountMetas,
        AccountDeserialize,
    },
    litesvm::LiteSVM,
    solana_account::Account as SolanaAccount,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_keypair::Keypair,
    solana_transaction::versioned::VersionedTransaction,
    flash_pool::state::FlashPool,
};

#[test]
fn test_full_lifecycle_flow() {
    let program_id = flash_pool::id();
    let payer = Keypair::new();
    let user1 = Keypair::new();
    let user2 = Keypair::new();
    let oracle_feed = Pubkey::new_unique();
    let base_price: u64 = 95_000; 
    let precision_step: u64 = 10; 
    let mint_keypair = Keypair::new();
    let mint_pubkey = mint_keypair.pubkey();
    let treasury = Keypair::new();

    let mut svm = LiteSVM::new();
    let bytes = include_bytes!("../../../target/deploy/flash_pool.so");
    svm.add_program(program_id, bytes).unwrap();
    svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();
    svm.airdrop(&user1.pubkey(), 10_000_000_000).unwrap();
    svm.airdrop(&user2.pubkey(), 10_000_000_000).unwrap();

    // 1. Setup Mint
    let rent = svm.minimum_balance_for_rent_exemption(82);
    let create_mint_ix = anchor_lang::solana_program::system_instruction::create_account(&payer.pubkey(), &mint_pubkey, rent, 82, &anchor_spl::token_2022::ID);
    let init_mint_ix = anchor_spl::token_2022::spl_token_2022::instruction::initialize_mint(&anchor_spl::token_2022::ID, &mint_pubkey, &payer.pubkey(), None, 6).unwrap();
    let msg = Message::new_with_blockhash(&[create_mint_ix, init_mint_ix], Some(&payer.pubkey()), &svm.latest_blockhash());
    svm.send_transaction(VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[&payer, &mint_keypair]).unwrap()).unwrap();

    // 2. Initialize Pool
    let (flash_pool_pda, _bump) = Pubkey::find_program_address(&[b"flash_pool", oracle_feed.as_ref()], &program_id);
    let (vault_pda, _vault_bump) = Pubkey::find_program_address(&[b"vault", flash_pool_pda.as_ref()], &program_id);
    let init_pool_ix = Instruction::new_with_bytes(program_id, &flash_pool::instruction::InitializePool { oracle_feed, base_price, precision_step }.data(), flash_pool::accounts::InitializePool { flash_pool: flash_pool_pda, vault: vault_pda, mint: mint_pubkey, payer: payer.pubkey(), token_program: anchor_spl::token_2022::ID, system_program: system_program::ID, rent: anchor_lang::prelude::rent::ID }.to_account_metas(None));
    svm.send_transaction(VersionedTransaction::try_new(VersionedMessage::Legacy(Message::new_with_blockhash(&[init_pool_ix], Some(&payer.pubkey()), &svm.latest_blockhash())), &[&payer]).unwrap()).unwrap();

    // 3. Setup User ATAs and Mint Tokens
    let setup_user = |svm: &mut LiteSVM, user: &Keypair, payer: &Keypair, mint: &Pubkey| {
        let ata_keypair = Keypair::new();
        let rent = svm.minimum_balance_for_rent_exemption(165);
        let ix1 = anchor_lang::solana_program::system_instruction::create_account(&payer.pubkey(), &ata_keypair.pubkey(), rent, 165, &anchor_spl::token_2022::ID);
        let ix2 = anchor_spl::token_2022::spl_token_2022::instruction::initialize_account(&anchor_spl::token_2022::ID, &ata_keypair.pubkey(), mint, &user.pubkey()).unwrap();
        let ix3 = anchor_spl::token_2022::spl_token_2022::instruction::mint_to(&anchor_spl::token_2022::ID, mint, &ata_keypair.pubkey(), &payer.pubkey(), &[], 10_000_000).unwrap();
        let msg = Message::new_with_blockhash(&[ix1, ix2, ix3], Some(&payer.pubkey()), &svm.latest_blockhash());
        svm.send_transaction(VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[payer, &ata_keypair]).unwrap()).unwrap();
        ata_keypair.pubkey()
    };
    let user1_ata = setup_user(&mut svm, &user1, &payer, &mint_pubkey);
    let user2_ata = setup_user(&mut svm, &user2, &payer, &mint_pubkey);

    // 4. Place Predictions
    // User 1: Bucket 15 ($951.50)
    // User 2: Bucket 17 ($951.70)
    let place_pred = |svm: &mut LiteSVM, user: &Keypair, flash_pool: Pubkey, vault: Pubkey, mint: Pubkey, user_ata: Pubkey, val: u64| {
        let (up_pda, _) = Pubkey::find_program_address(&[b"user_prediction", flash_pool.as_ref(), user.pubkey().as_ref()], &program_id);
        let ix = Instruction::new_with_bytes(program_id, &flash_pool::instruction::PlacePrediction { prediction_value: val }.data(), flash_pool::accounts::PlacePrediction { flash_pool, user_prediction: up_pda, vault, mint, user_token_account: user_ata, user: user.pubkey(), token_program: anchor_spl::token_2022::ID, system_program: system_program::ID }.to_account_metas(None));
        svm.send_transaction(VersionedTransaction::try_new(VersionedMessage::Legacy(Message::new_with_blockhash(&[ix], Some(&user.pubkey()), &svm.latest_blockhash())), &[user]).unwrap()).unwrap();
        up_pda
    };
    let user1_up = place_pred(&mut svm, &user1, flash_pool_pda, vault_pda, mint_pubkey, user1_ata, 95_150);
    let user2_up = place_pred(&mut svm, &user2, flash_pool_pda, vault_pda, mint_pubkey, user2_ata, 95_170);

    // 5. Resolve Market
    // We inject a mock PriceUpdateV2 account into LiteSVM memory.
    // The byte layout matches the Pyth SDK's PriceUpdateV2 struct exactly.
    // See resolve_market.rs for the full offset map.
    //
    // Outcome price: $951.60 → base_price=95_000, precision_step=10
    // In Pyth format: raw_price = 9516, exponent = -2
    //   oracle_price = 9516 / 10^(2-2) = 9516 ... that's wrong for 95160.
    // Let's use:  raw_price = 9_516_000_000, exponent = -8
    //   scale = 10^(8-2) = 10^6 = 1_000_000
    //   oracle_price = 9_516_000_000 / 1_000_000 = 9_516  -- hmm still wrong
    //
    // Correction: base_price=95_000 means $950.00 with 2 decimal places.
    //   We need oracle_price = 95_160 (= $951.60 in our 2-decimal format).
    //   Pyth raw_price in units 10^-8:  95_160 * 10^6 = 95_160_000_000 with exp=-8
    //   scale = 10^(8-2) = 1_000_000
    //   oracle_price = 95_160_000_000 / 1_000_000 = 95_160 ✓
    let mut oracle_data = vec![0u8; 160];

    // [0..8]   Discriminator (we own the account, owner check is what matters)
    oracle_data[0..8].copy_from_slice(&[0u8; 8]);

    // [8..40]  write_authority: any Pubkey
    oracle_data[8..40].copy_from_slice(&[1u8; 32]);

    // [40..42] verification_level: Full = variant tag 1, no extra byte
    oracle_data[40] = 1; // VerificationLevel::Full
    oracle_data[41] = 0; // (second byte of 2-byte fixed encoding)

    // [42..74] feed_id: SOL/USD feed id
    let sol_usd_feed_id: [u8; 32] = [
        0xef, 0x0d, 0x8b, 0x6f, 0xda, 0x2c, 0xeb, 0xa4,
        0x1d, 0xa1, 0x5d, 0x40, 0x95, 0xd1, 0xda, 0x39,
        0x2a, 0x0d, 0x2f, 0x8e, 0xd0, 0xc6, 0xc7, 0xbc,
        0x0f, 0x4c, 0xfa, 0xc8, 0xc2, 0x80, 0xb5, 0x6d,
    ];
    oracle_data[42..74].copy_from_slice(&sol_usd_feed_id);

    // [74..82] price: i64 = 95_160_000_000 (raw Pyth units)
    let raw_price: i64 = 95_160_000_000;
    oracle_data[74..82].copy_from_slice(&raw_price.to_le_bytes());

    // [90..94] exponent: i32 = -8
    let exponent: i32 = -8;
    oracle_data[90..94].copy_from_slice(&exponent.to_le_bytes());

    // [94..102] publish_time: set to current LiteSVM clock (always fresh)
    let publish_time: i64 = 0; // LiteSVM clock is at unix timestamp 0 by default
    oracle_data[94..102].copy_from_slice(&publish_time.to_le_bytes());

    let price_update_key = Pubkey::new_unique();
    let pyth_receiver_id: Pubkey = "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ".parse().unwrap();
    let rent_lamports = svm.minimum_balance_for_rent_exemption(oracle_data.len());
    svm.set_account(
        price_update_key,
        SolanaAccount {
            lamports: rent_lamports,
            data: oracle_data,
            owner: pyth_receiver_id,
            executable: false,
            rent_epoch: 0,
        },
    ).unwrap();

    let resolve_ix = Instruction::new_with_bytes(
        program_id,
        &flash_pool::instruction::ResolveMarket {}.data(),
        flash_pool::accounts::ResolveMarket {
            flash_pool: flash_pool_pda,
            price_update: price_update_key,
            payer: payer.pubkey(),
        }.to_account_metas(None),
    );
    svm.send_transaction(VersionedTransaction::try_new(VersionedMessage::Legacy(Message::new_with_blockhash(&[resolve_ix], Some(&payer.pubkey()), &svm.latest_blockhash())), &[&payer]).unwrap()).unwrap();

    // Verify: outcome should be 95_160 ($951.60 in 2-decimal format)
    let pool_acc = svm.get_account(&flash_pool_pda).unwrap();
    let pool_state = FlashPool::try_deserialize(&mut &pool_acc.data[..]).unwrap();
    assert_eq!(pool_state.median_error, 1);
    assert_eq!(pool_state.outcome, 95_160);

    // 6. Claim Rewards
    let claim_reward = |svm: &mut LiteSVM, user: &Keypair, user_up: Pubkey, flash_pool: Pubkey, vault: Pubkey, mint: Pubkey, user_ata: Pubkey, treasury: Pubkey| {
        let ix = Instruction::new_with_bytes(program_id, &flash_pool::instruction::ClaimReward {}.data(), flash_pool::accounts::ClaimReward { flash_pool, user_prediction: user_up, vault, mint, user_token_account: user_ata, user: user.pubkey(), treasury, payer: payer.pubkey(), token_program: anchor_spl::token_2022::ID, system_program: system_program::ID }.to_account_metas(None));
        svm.send_transaction(VersionedTransaction::try_new(VersionedMessage::Legacy(Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &svm.latest_blockhash())), &[&payer]).unwrap())
    };

    // User 1 claims
    let res1 = claim_reward(&mut svm, &user1, user1_up, flash_pool_pda, vault_pda, mint_pubkey, user1_ata, treasury.pubkey());
    assert!(res1.is_ok(), "User 1 claim failed: {:?}", res1.err());

    // User 2 claims
    let res2 = claim_reward(&mut svm, &user2, user2_up, flash_pool_pda, vault_pda, mint_pubkey, user2_ata, treasury.pubkey());
    assert!(res2.is_ok(), "User 2 claim failed: {:?}", res2.err());

    // Verify Payouts (total pool was 2,000,000. Payout per user = 2,000,000 / 2 = 1,000,000)
    let user1_ata_acc = svm.get_account(&user1_ata).unwrap();
    let user1_amount = u64::from_le_bytes(user1_ata_acc.data[64..72].try_into().unwrap());
    // Initial 10,000,000 - 1,000,000 (fee) + 1,000,000 (payout) = 10,000,000
    assert_eq!(user1_amount, 10_000_000);

    // User Prediction accounts should be closed
    assert!(svm.get_account(&user1_up).is_none());
    assert!(svm.get_account(&user2_up).is_none());
}
