
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
fn test_initialize_pool_success() {
    let program_id = flash_pool::id();
    let payer = Keypair::new();
    let oracle_feed = Pubkey::new_unique();
    let base_price: u64 = 95_000;
    let precision_step: u64 = 10;
    let mint_keypair = Keypair::new();
    let mint_pubkey = mint_keypair.pubkey();

    let mut svm = LiteSVM::new();
    let bytes = include_bytes!("../../../target/deploy/flash_pool.so");
    svm.add_program(program_id, bytes).unwrap();
    svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap(); // 10 SOL

    // 1. Initialize the USDC Mint (Token-2022)
    // We'll just add the account directly for simplicity in LiteSVM if possible,
    // or send a transaction. Let's try adding it directly with basic Mint data.
    // A mint account size is 82 bytes for standard Token.
    // For Token-2022 it can be more, but let's use standard for now.
    
    // Instead of manual account creation, let's just use a dummy address for mint 
    // and see if the program's 'init' for vault works with a non-existent mint (it won't).
    // So we MUST create the mint.
    
    let (flash_pool_pda, _bump) = Pubkey::find_program_address(
        &[b"flash_pool", oracle_feed.as_ref()],
        &program_id,
    );

    let (vault_pda, _vault_bump) = Pubkey::find_program_address(
        &[b"vault", flash_pool_pda.as_ref()],
        &program_id,
    );

    // To properly test, we need to initialize the mint account.
    // LiteSVM doesn't have a "create_mint" helper yet, so we send instructions.
    
    let rent = svm.minimum_balance_for_rent_exemption(82); // Mint size
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

    // 2. Initialize the Pool
    let instruction = Instruction::new_with_bytes(
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
    let msg = Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[&payer]).unwrap();

    let res = svm.send_transaction(tx);
    assert!(res.is_ok(), "InitializePool failed: {:?}", res.err());

    // 3. Verify State
    let account = svm.get_account(&flash_pool_pda).expect("FlashPool account not found");
    let pool_state = FlashPool::try_deserialize(&mut &account.data[..]).expect("Failed to deserialize FlashPool");

    assert_eq!(pool_state.oracle_feed, oracle_feed);
    assert_eq!(pool_state.base_price, base_price);
    assert_eq!(pool_state.precision_step, precision_step);
    assert_eq!(pool_state.total_participants, 0);
    assert_eq!(pool_state.total_pool_amount, 0);
    assert_eq!(pool_state.outcome, 0);
}
