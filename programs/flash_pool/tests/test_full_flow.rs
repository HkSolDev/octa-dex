
use {
    anchor_lang::{
        prelude::Pubkey,
        system_program,
        solana_program::instruction::Instruction,
        InstructionData, ToAccountMetas,
        AccountDeserialize,
    },
    litesvm::LiteSVM,
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
    // Outcome: $951.60 -> Bucket 16
    // Median error loop:
    // - i=15: count=1, total=1, target=1. Loop breaks.
    // - winning_bucket = 16.
    // - median_error = |15 - 16| = 1.
    // Winners: Buckets 15, 16, 17. Both users win!
    let resolve_ix = Instruction::new_with_bytes(program_id, &flash_pool::instruction::ResolveMarket { oracle_price: 95_160 }.data(), flash_pool::accounts::ResolveMarket { flash_pool: flash_pool_pda, payer: payer.pubkey() }.to_account_metas(None));
    svm.send_transaction(VersionedTransaction::try_new(VersionedMessage::Legacy(Message::new_with_blockhash(&[resolve_ix], Some(&payer.pubkey()), &svm.latest_blockhash())), &[&payer]).unwrap()).unwrap();

    // Verify median_error
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
