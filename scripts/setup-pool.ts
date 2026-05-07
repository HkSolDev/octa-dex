/**
 * ⚡ FlashPool — Phase 3: Pool Setup & MagicBlock Delegation
 *
 * Step 1 — create Token-2022 test mint (if first run).
 * Step 2 — initialize_pool:   creates FlashPool PDA + Vault on Devnet base layer.
 * Step 3 — delegate:          locks the FlashPool PDA to the MagicBlock ER so
 *                              place_prediction txs run at 10–50ms, zero gas.
 *
 * Run ONCE per round (before the 60-second window opens):
 *   npm run setup
 */

import * as anchor from "@coral-xyz/anchor";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
} from "@solana/web3.js";
import {
  TOKEN_2022_PROGRAM_ID,
  createMint,
  getOrCreateAssociatedTokenAccount,
  mintTo,
} from "@solana/spl-token";
import * as dotenv from "dotenv";
import * as fs from "fs";
import * as path from "path";

dotenv.config();

// ─── Network ───────────────────────────────────────────────────────────────────
const DEVNET_RPC        = "https://api.devnet.solana.com";
const MAGIC_ROUTER_RPC  = "https://devnet-rpc.magicblock.app";

// ─── Fixed Addresses ───────────────────────────────────────────────────────────
const DELEGATION_PROGRAM_ID = new PublicKey("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");
const ER_VALIDATOR_KEY      = new PublicKey("MUS3hc9TCw4cGC12vHNoYcCGzJG1txjgQLZWVoeNHNd");

/**
 * Pyth Lazer BTC/USD feed on MagicBlock Devnet.
 * The MagicBlock chain-pusher writes fresh price data here every ~50ms.
 * Used as the oracle_feed when initializing the pool.
 */
const PYTH_LAZER_FEED = new PublicKey("71wtTRDY8Gxgw56bXFt2oc6qeAbTxzStdNiC425Z51sr");

/**
 * Path to the file where we cache the Token-2022 test mint address between runs.
 * The mint is created once and reused forever.
 */
const MINT_CACHE_FILE = path.resolve(__dirname, "../frontend/.env");

// ─── Pool Parameters ───────────────────────────────────────────────────────────
// BTC/USD around $100k: base = $99,500, step = $0.99 → 100 buckets cover ~$99
const BASE_PRICE      = new anchor.BN("9950000"); // $99,500.00 (2-decimal integer)
const PRECISION_STEP  = new anchor.BN("99");      // $0.99 per bucket

// ─── Seeds ─────────────────────────────────────────────────────────────────────
const SEED_FLASH_POOL = Buffer.from("flash_pool");
const SEED_VAULT      = Buffer.from("vault");

// ─── Helpers ───────────────────────────────────────────────────────────────────
function loadKeypair(filePath: string): Keypair {
  const expanded = filePath.replace("~", process.env.HOME ?? "");
  const raw = fs.readFileSync(path.resolve(expanded), "utf-8");
  return Keypair.fromSecretKey(new Uint8Array(JSON.parse(raw) as number[]));
}

function deriveFlashPool(oracleFeed: PublicKey, programId: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [SEED_FLASH_POOL, oracleFeed.toBuffer()],
    programId
  );
}

function deriveVault(flashPool: PublicKey, programId: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [SEED_VAULT, flashPool.toBuffer()],
    programId
  );
}

// ─── Main ──────────────────────────────────────────────────────────────────────
async function main() {
  console.log("⚡ FlashPool Phase 3 — Setup & Delegation");
  console.log("─".repeat(55));

  const payer = loadKeypair("~/.config/solana/id.json");
  console.log(`Admin wallet : ${payer.publicKey.toBase58()}`);

  // Load IDL — Anchor 0.30 embeds the programId inside the IDL json
  const idlPath = path.resolve(__dirname, "../target/idl/flash_pool.json");
  if (!fs.existsSync(idlPath)) {
    throw new Error("IDL not found. Run `anchor build` first.");
  }
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const idl: any = JSON.parse(fs.readFileSync(idlPath, "utf-8"));

  // Derive program ID from the deploy keypair
  const deployKeyPath = path.resolve(__dirname, "../target/deploy/flash_pool-keypair.json");
  const deployKey = Keypair.fromSecretKey(
    new Uint8Array(JSON.parse(fs.readFileSync(deployKeyPath, "utf-8")) as number[])
  );
  const PROGRAM_ID = deployKey.publicKey;
  console.log(`Program ID   : ${PROGRAM_ID.toBase58()}`);

  // ── STEP 1: initialize_pool on Devnet base layer ──────────────────────────
  console.log("\n🔧 STEP 1: Initializing FlashPool on Solana Devnet...");

  const connection = new Connection(DEVNET_RPC, "confirmed");
  const wallet     = new anchor.Wallet(payer);
  const provider   = new anchor.AnchorProvider(connection, wallet, { commitment: "confirmed" });
  anchor.setProvider(provider);

  // ✅ Anchor 0.30 API: Program(idl, provider) — programId comes from idl.address
  // We patch the IDL address field to match our deploy keypair.
  idl.address = PROGRAM_ID.toBase58();
  const program = new anchor.Program(idl, provider);

  // ── STEP 1: Create (or reuse) Token-2022 test mint ───────────────────────
  console.log("\n🪙 STEP 1: Token-2022 test mint...");

  // Check if we already cached a mint in .env
  let usdcMint: PublicKey;
  const envContent = fs.existsSync(MINT_CACHE_FILE)
    ? fs.readFileSync(MINT_CACHE_FILE, "utf-8")
    : "";
  const existingMintMatch = envContent.match(/NEXT_PUBLIC_USDC_MINT="([^"]+)"/);

  if (existingMintMatch && existingMintMatch[1] !== "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU") {
    // Reuse the previously created Token-2022 mint
    usdcMint = new PublicKey(existingMintMatch[1]);
    console.log(`   ♻️  Reusing Token-2022 mint: ${usdcMint.toBase58()}`);
  } else {
    // Create a fresh Token-2022 USDC-like mint (6 decimals)
    console.log("   Creating new Token-2022 test mint (6 decimals)...");
    usdcMint = await createMint(
      connection,
      payer,                // fee payer
      payer.publicKey,      // mint authority
      payer.publicKey,      // freeze authority
      6,                    // decimals (USDC standard)
      undefined,            // keypair (auto-generated)
      undefined,            // confirmOptions
      TOKEN_2022_PROGRAM_ID // ← MUST be Token-2022 to match the Vault's program
    );
    console.log(`   ✅ Token-2022 mint created: ${usdcMint.toBase58()}`);

    // Persist the mint address into frontend/.env
    let newEnv = envContent.replace(
      /NEXT_PUBLIC_USDC_MINT="[^"]*"/,
      `NEXT_PUBLIC_USDC_MINT="${usdcMint.toBase58()}"`
    );
    if (!newEnv.includes("NEXT_PUBLIC_USDC_MINT")) {
      newEnv += `\nNEXT_PUBLIC_USDC_MINT="${usdcMint.toBase58()}"\n`;
    }
    fs.writeFileSync(MINT_CACHE_FILE, newEnv);
    console.log(`   💾 Saved to frontend/.env`);
  }

  // Mint 10,000 test tokens to the payer so they can fund bot wallets
  const payerAta = await getOrCreateAssociatedTokenAccount(
    connection, payer, usdcMint, payer.publicKey,
    false, "confirmed", {}, TOKEN_2022_PROGRAM_ID
  );
  await mintTo(
    connection, payer, usdcMint, payerAta.address, payer,
    10_000 * 1_000_000, // 10,000 USDC (6 decimals)
    [], undefined, TOKEN_2022_PROGRAM_ID
  );
  console.log(`   💰 Minted 10,000 test USDC to ${payerAta.address.toBase58()}`);

  // ── STEP 2: initialize_pool ────────────────────────────────────────────────

  const [flashPoolPda] = deriveFlashPool(PYTH_LAZER_FEED, PROGRAM_ID);
  const [vaultPda]     = deriveVault(flashPoolPda, PROGRAM_ID);

  console.log(`\n🔧 STEP 2: Initializing FlashPool on Solana Devnet...`);
  console.log(`   FlashPool PDA : ${flashPoolPda.toBase58()}`);
  console.log(`   Vault PDA     : ${vaultPda.toBase58()}`);
  console.log(`   Oracle Feed   : ${PYTH_LAZER_FEED.toBase58()}`);
  console.log(`   Base Price    : $${(BASE_PRICE.toNumber() / 100).toFixed(2)}`);
  console.log(`   Step          : $${(PRECISION_STEP.toNumber() / 100).toFixed(2)} / bucket`);
  console.log(`   USDC Mint     : ${usdcMint.toBase58()}`);

  // Update .env with the derived PDA addresses
  let envFile = fs.readFileSync(MINT_CACHE_FILE, "utf-8");
  envFile = envFile
    .replace(/NEXT_PUBLIC_FLASH_POOL_PDA="[^"]*"/, `NEXT_PUBLIC_FLASH_POOL_PDA="${flashPoolPda.toBase58()}"`)
    .replace(/NEXT_PUBLIC_VAULT_PDA="[^"]*"/, `NEXT_PUBLIC_VAULT_PDA="${vaultPda.toBase58()}"`);
  fs.writeFileSync(MINT_CACHE_FILE, envFile);

  // Skip if already initialised
  const existing = await connection.getAccountInfo(flashPoolPda);
  if (existing) {
    console.log("\n   ⚠️  FlashPool PDA already exists — skipping initialize_pool.");
    console.log("   ℹ️  To start a fresh round, change PYTH_LAZER_FEED address.");
  } else {
    const tx = await (program.methods as any)
      .initializePool(PYTH_LAZER_FEED, BASE_PRICE, PRECISION_STEP)
      .accounts({
        flashPool:     flashPoolPda,
        vault:         vaultPda,
        mint:          usdcMint,
        payer:         payer.publicKey,
        tokenProgram:  TOKEN_2022_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent:          SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    console.log(`\n   ✅ initialize_pool tx : ${tx}`);
    console.log(`   🔗 https://explorer.solana.com/tx/${tx}?cluster=devnet`);
  }

  // ── STEP 3: Delegate FlashPool PDA via on-chain CPI ───────────────────────
  //
  // FlashPool is a PDA — it has no private key so an off-chain script cannot
  // sign for it directly. The `delegate_pool_to_er` instruction in our program
  // uses invoke_signed with the FlashPool's PDA seeds, which is the only
  // valid way to authorize the delegation on Solana.
  //
  // All MagicBlock Delegation Program accounts are derived + passed here.
  // The on-chain Rust code handles all the raw byte encoding.

  console.log("\n🚀 STEP 3: Delegating FlashPool to MagicBlock ER (Opaque CPI Proxy)...");
  console.log(`   ER Validator : ${ER_VALIDATOR_KEY.toBase58()}`);

  // ── Let the JS SDK build the perfect v0.13 delegation instruction ─────────
  // This gives us the exactly-right account layout and serialized data payload
  // regardless of which SDK version MagicBlock is running on-chain.
  const { createDelegateInstruction, DELEGATION_PROGRAM_ID: SDK_DELEGATION_PROGRAM_ID } =
    require("@magicblock-labs/ephemeral-rollups-sdk");

  const delegateIx = createDelegateInstruction(
    {
      payer:            payer.publicKey,
      delegatedAccount: flashPoolPda,
      ownerProgram:     PROGRAM_ID,
      validator:        ER_VALIDATOR_KEY,
    },
    { commitFrequencyMs: 500 }
  );

  console.log(`   SDK-generated keys (${delegateIx.keys.length}):`);
  delegateIx.keys.forEach((k: any, i: number) =>
    console.log(`     [${i}] ${k.pubkey.toBase58()} w:${k.isWritable} s:${k.isSigner}`)
  );

  // ── Map keys → remainingAccounts, stripping PDA isSigner ─────────────────
  // The PDA can't be signed off-chain. Our Rust proxy elevates it to signer
  // on-chain via invoke_signed with the PDA's seeds.
  const remainingAccounts = delegateIx.keys.map((meta: any) => ({
    pubkey:     meta.pubkey,
    isSigner:   meta.pubkey.equals(flashPoolPda) ? false : meta.isSigner,
    isWritable: meta.isWritable,
  }));

  // ── Append the Delegation Program itself ──────────────────────────────────
  // invoke_signed requires the invoked program's AccountInfo in the account_infos
  // slice. The JS SDK puts it in programId, not keys — so we add it manually.
  remainingAccounts.push({
    pubkey:     DELEGATION_PROGRAM_ID,
    isSigner:   false,
    isWritable: false,
  });

  console.log(`\n   Sending delegateProxy tx...`);

  const delegateTx = await (program.methods as any)
    .delegateProxy(Buffer.from(delegateIx.data))
    .accounts({
      payer:             payer.publicKey,
      oracleFeed:        PYTH_LAZER_FEED,
      flashPool:         flashPoolPda,
      delegationProgram: DELEGATION_PROGRAM_ID,
    })
    .remainingAccounts(remainingAccounts)
    .rpc();

  console.log(`\n   ✅ delegate tx : ${delegateTx}`);
  console.log(`   🔗 https://explorer.solana.com/tx/${delegateTx}?cluster=devnet`);
  console.log(`\n   🔒 FlashPool PDA is now LOCKED on L1.`);
  console.log(`   All place_prediction txs must now go through the Magic Router.`);


  // ── Summary ───────────────────────────────────────────────────────────────
  console.log("\n" + "═".repeat(55));
  console.log("✅ SETUP COMPLETE");
  console.log("═".repeat(55));
  console.log(`FlashPool PDA : ${flashPoolPda.toBase58()}`);
  console.log(`Vault PDA     : ${vaultPda.toBase58()}`);
  console.log(`Oracle Feed   : ${PYTH_LAZER_FEED.toBase58()}`);
  console.log("");
  console.log("Next steps:");
  console.log("  1. Paste the FlashPool PDA into frontend/.env → NEXT_PUBLIC_FLASH_POOL_PDA");
  console.log("  2. Run: npm run bots   ← fires 1,000 predictions via Magic Router");
  console.log("  3. Watch the React histogram fill in real-time!");
}

main().catch(err => {
  console.error("\n❌ Fatal error:", err);
  process.exit(1);
});
