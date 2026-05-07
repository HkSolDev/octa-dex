
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
    flash_pool::state::{FlashPool, UserPrediction},
    anchor_spl::token_interface::{Mint, TokenAccount},
};

#[test]
fn test_prediction_cycle() {
    let program_id = flash_pool::id();
    let payer = Keypair::new();
    let user = Keypair::new();
    let oracle_feed = Pubkey::new_unique();
    let base_price: u64 = 95_000; // e.g. $950.00
    let precision_step: u64 = 10; // $0.10 buckets
    let mint_keypair = Keypair::new();
    let mint_pubkey = mint_keypair.pubkey();

    let mut svm = LiteSVM::new();
    let bytes = include_bytes!("../../../target/deploy/flash_pool.so");
    svm.add_program(program_id, bytes).unwrap();
    svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();
    svm.airdrop(&user.pubkey(), 10_000_000_000).unwrap();

    // 1. Setup Mint (Token-2022)
    let rent = svm.minimum_balance_for_rent_exemption(82);
    let create_mint_ix = anchor_lang::solana_program::system_instruction::create_account(
        &payer.pubkey(),
        &mint_pubkey,
        rent,
        82,
        &anchor_spl::token_2022::ID,
    );
    let init_mint_ix = anchor_spl::token_2022::spl_token_2022::instruction::initialize_mint(
        &anchor_spl::token_2022::ID,
        &mint_pubkey,
        &payer.pubkey(),
        None,
        6,
    ).unwrap();

    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[create_mint_ix, init_mint_ix], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[&payer, &mint_keypair]).unwrap();
    svm.send_transaction(tx).unwrap();

    // 2. Initialize Pool
    let (flash_pool_pda, _bump) = Pubkey::find_program_address(
        &[b"flash_pool", oracle_feed.as_ref()],
        &program_id,
    );
    let (vault_pda, _vault_bump) = Pubkey::find_program_address(
        &[b"vault", flash_pool_pda.as_ref()],
        &program_id,
    );

    let init_pool_ix = Instruction::new_with_bytes(
        program_id,
        &flash_pool::instruction::InitializePool {
            oracle_feed,
            base_price,
            precision_step,
        }.data(),
        flash_pool::accounts::InitializePool {
            flash_pool: flash_pool_pda,
            vault: vault_pda,
            mint: mint_pubkey,
            payer: payer.pubkey(),
            token_program: anchor_spl::token_2022::ID,
            system_program: system_program::ID,
            rent: anchor_lang::prelude::rent::ID,
        }.to_account_metas(None),
    );

    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[init_pool_ix], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[&payer]).unwrap();
    svm.send_transaction(tx).unwrap();

    // 3. Setup User Token Account & Mint Tokens
    let user_ata_keypair = Keypair::new();
    let user_ata = user_ata_keypair.pubkey();
    let rent = svm.minimum_balance_for_rent_exemption(165); // TokenAccount size
    let create_user_ata_ix = anchor_lang::solana_program::system_instruction::create_account(
        &payer.pubkey(),
        &user_ata,
        rent,
        165,
        &anchor_spl::token_2022::ID,
    );
    let init_user_ata_ix = anchor_spl::token_2022::spl_token_2022::instruction::initialize_account(
        &anchor_spl::token_2022::ID,
        &user_ata,
        &mint_pubkey,
        &user.pubkey(),
    ).unwrap();
    let mint_to_user_ix = anchor_spl::token_2022::spl_token_2022::instruction::mint_to(
        &anchor_spl::token_2022::ID,
        &mint_pubkey,
        &user_ata,
        &payer.pubkey(),
        &[],
        10_000_000, // 10 USDC (assuming 6 decimals)
    ).unwrap();

    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[create_user_ata_ix, init_user_ata_ix, mint_to_user_ix], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[&payer, &user_ata_keypair]).unwrap();
    svm.send_transaction(tx).unwrap();

    // 4. Place Prediction
    let prediction_value: u64 = 95_150; // $951.50 -> should go to bucket 15
    let (user_prediction_pda, _up_bump) = Pubkey::find_program_address(
        &[b"user_prediction", flash_pool_pda.as_ref(), user.pubkey().as_ref()],
        &program_id,
    );

    let place_ix = Instruction::new_with_bytes(
        program_id,
        &flash_pool::instruction::PlacePrediction {
            prediction_value,
        }.data(),
        flash_pool::accounts::PlacePrediction {
            flash_pool: flash_pool_pda,
            user_prediction: user_prediction_pda,
            vault: vault_pda,
            mint: mint_pubkey,
            user_token_account: user_ata,
            user: user.pubkey(),
            token_program: anchor_spl::token_2022::ID,
            system_program: system_program::ID,
        }.to_account_metas(None),
    );

    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[place_ix], Some(&user.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[&user]).unwrap();

    let res = svm.send_transaction(tx);
    assert!(res.is_ok(), "PlacePrediction failed: {:?}", res.err());

    // 5. Verify Results
    let pool_acc = svm.get_account(&flash_pool_pda).unwrap();
    let pool_state = FlashPool::try_deserialize(&mut &pool_acc.data[..]).unwrap();
    assert_eq!(pool_state.total_participants, 1);
    assert_eq!(pool_state.total_pool_amount, 1_000_000); // 1 USDC fee
    assert_eq!(pool_state.histogram_buckets[15], 1);

    let up_acc = svm.get_account(&user_prediction_pda).unwrap();
    let up_state = UserPrediction::try_deserialize(&mut &up_acc.data[..]).unwrap();
    assert_eq!(up_state.user, user.pubkey());
    assert_eq!(up_state.predicted_bucket_index, 15);
    
    // Vault balance should be 1 USDC
    let vault_acc = svm.get_account(&vault_pda).unwrap();
    // In Token-2022, the amount is at bytes 64-72
    let amount = u64::from_le_bytes(vault_acc.data[64..72].try_into().unwrap());
    assert_eq!(amount, 1_000_000);
}
