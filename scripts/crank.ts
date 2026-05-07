/**
 * ⚡ FlashPool — Phase 3: Crank (Market Resolution + Winner Payouts)
 *
 * This script runs AFTER the 60-second prediction window closes.
 * It does:
 *   1. Calls `resolve_market` on the ER (via Magic Router) to read the
 *      Pyth Lazer oracle price and calculate the median error.
 *   2. The MagicBlock sequencer auto-commits the resolved state back to L1.
 *   3. Fetches all UserPrediction PDAs for the pool.
 *   4. Calls `claim_reward` for each winner in batches.
 *
 * Run AFTER the 60s round timer expires:
 *   npx ts-node scripts/crank.ts
 */

import * as anchor from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import { TOKEN_2022_PROGRAM_ID, getOrCreateAssociatedTokenAccount } from "@solana/spl-token";
import * as dotenv from "dotenv";
import * as fs from "fs";
import * as path from "path";

dotenv.config();

// ─── Config ────────────────────────────────────────────────────────────────────

/**
 * Use Magic Router to call resolve_market while the pool is still delegated.
 * After the MagicBlock sequencer undelegates (auto-commit), you can switch
 * to standard Devnet RPC for claim_reward calls.
 */
const MAGIC_ROUTER_RPC = "https://devnet-rpc.magicblock.app";
const DEVNET_RPC = "https://api.devnet.solana.com";

// ─── Seeds ─────────────────────────────────────────────────────────────────────

const SEED_FLASH_POOL = Buffer.from("flash_pool");
const SEED_USER_PREDICTION = Buffer.from("user_prediction");
const SEED_VAULT = Buffer.from("vault");

// ─── Helpers ───────────────────────────────────────────────────────────────────

function loadKeypair(filePath: string): Keypair {
  const expanded = filePath.replace("~", process.env.HOME ?? "");
  const raw = fs.readFileSync(path.resolve(expanded), "utf-8");
  return Keypair.fromSecretKey(new Uint8Array(JSON.parse(raw)));
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

// ─── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  console.log("⚡ FlashPool Phase 3 — Crank (Resolve + Payouts)");
  console.log("─".repeat(55));

  const feePayer = loadKeypair("~/.config/solana/id.json");
  const treasury = feePayer.publicKey; // rent from closed PDAs goes here

  // Load IDL and program
  const idlPath = path.resolve(__dirname, "../target/idl/flash_pool.json");
  const idl = JSON.parse(fs.readFileSync(idlPath, "utf-8"));
  const deployKeyPath = path.resolve(__dirname, "../target/deploy/flash_pool-keypair.json");
  const deployKey = Keypair.fromSecretKey(
    new Uint8Array(JSON.parse(fs.readFileSync(deployKeyPath, "utf-8")))
  );
  const PROGRAM_ID = deployKey.publicKey;

  const oracleFeed = new PublicKey(
    process.env.ORACLE_FEED ?? "71wtTRDY8Gxgw56bXFt2oc6qeAbTxzStdNiC425Z51sr"
  );
  const USDC_MINT = new PublicKey("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");

  const [flashPoolPda] = PublicKey.findProgramAddressSync(
    [SEED_FLASH_POOL, oracleFeed.toBuffer()],
    PROGRAM_ID
  );
  const [vaultPda] = PublicKey.findProgramAddressSync(
    [SEED_VAULT, flashPoolPda.toBuffer()],
    PROGRAM_ID
  );

  // ── STEP 1: Resolve the market ─────────────────────────────────────────────

  console.log("\n🔮 STEP 1: Resolving market via Magic Router...");

  const erConnection = new Connection(MAGIC_ROUTER_RPC, "confirmed");
  const erProvider = new anchor.AnchorProvider(
    erConnection,
    new anchor.Wallet(feePayer),
    { commitment: "confirmed" }
  );
  anchor.setProvider(erProvider);
  const erProgram = new anchor.Program(idl, PROGRAM_ID, erProvider);

  const resolveTx = await erProgram.methods
    .resolveMarket()
    .accounts({
      flashPool: flashPoolPda,
      priceUpdate: oracleFeed, // The Pyth Lazer account on the ER
      payer: feePayer.publicKey,
    })
    .rpc();

  console.log(`✅ resolve_market tx: ${resolveTx}`);
  console.log(`🔗 https://explorer.solana.com/tx/${resolveTx}?cluster=devnet`);

  // Wait for MagicBlock sequencer to commit state back to L1
  console.log("\n⏳ Waiting for MagicBlock sequencer to commit state to L1...");
  await sleep(5000); // give it 5 seconds for the auto-commit

  // ── STEP 2: Read resolved state from L1 ───────────────────────────────────

  console.log("\n📖 STEP 2: Reading resolved pool state from Devnet L1...");

  const l1Connection = new Connection(DEVNET_RPC, "confirmed");
  const l1Provider = new anchor.AnchorProvider(
    l1Connection,
    new anchor.Wallet(feePayer),
    { commitment: "confirmed" }
  );
  const l1Program = new anchor.Program(idl, PROGRAM_ID, l1Provider);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const poolState = await (l1Program.account as any).flashPool.fetch(flashPoolPda);
  const outcome: bigint = BigInt(poolState.outcome.toString());
  const medianError: bigint = BigInt(poolState.medianError.toString());
  const basePrice: bigint = BigInt(poolState.basePrice.toString());
  const precisionStep: bigint = BigInt(poolState.precisionStep.toString());
  const totalParticipants: number = poolState.totalParticipants;

  console.log(`  Oracle outcome:  $${(Number(outcome) / 100).toFixed(2)}`);
  console.log(`  Median error:    ±${medianError} buckets`);
  console.log(`  Total players:   ${totalParticipants}`);

  if (outcome === BigInt(0)) {
    console.error("❌ Outcome is 0 — market has not been resolved yet.");
    process.exit(1);
  }

  // Calculate winning bucket
  const winBucket =
    outcome < basePrice
      ? BigInt(0)
      : (() => {
          const idx = (outcome - basePrice) / precisionStep;
          return idx >= BigInt(99) ? BigInt(99) : idx;
        })();

  console.log(`  Winning bucket:  ${winBucket}`);

  // ── STEP 3: Fetch all UserPrediction PDAs ─────────────────────────────────

  console.log("\n🔍 STEP 3: Fetching all UserPrediction PDAs...");

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const allPredictions = await (l1Program.account as any).userPrediction.all([
    {
      memcmp: {
        offset: 8 + 32, // skip discriminator + user pubkey to get pool field
        bytes: flashPoolPda.toBase58(),
      },
    },
  ]);

  console.log(`  Found ${allPredictions.length} UserPrediction PDAs`);

  // Filter winners
  const winners = allPredictions.filter((p: { account: { predictedBucketIndex: number } }) => {
    const userBucket = BigInt(p.account.predictedBucketIndex);
    const distance = userBucket > winBucket
      ? userBucket - winBucket
      : winBucket - userBucket;
    return distance <= medianError;
  });

  console.log(`  Winners: ${winners.length} players qualify for payout`);

  if (winners.length === 0) {
    console.log("  ℹ️  No winners found — pool funds remain in vault.");
    return;
  }

  // ── STEP 4: Claim rewards for all winners ─────────────────────────────────

  console.log(`\n💸 STEP 4: Paying out ${winners.length} winners...`);

  let paidCount = 0;
  const BATCH = 15; // claim up to 15 winners per "wave" to avoid timeout

  for (let i = 0; i < winners.length; i += BATCH) {
    const batch = winners.slice(i, i + BATCH);

    const batchPromises = batch.map(async (prediction: {
      publicKey: PublicKey;
      account: { user: PublicKey; predictedBucketIndex: number };
    }) => {
      try {
        const userKey: PublicKey = prediction.account.user;
        const userPredictionPda = prediction.publicKey;

        // Get/create the winner's USDC token account
        const userTokenAccount = await getOrCreateAssociatedTokenAccount(
          l1Connection,
          feePayer,
          USDC_MINT,
          userKey,
          false,
          "confirmed",
          {},
          TOKEN_2022_PROGRAM_ID
        );

        const tx = await l1Program.methods
          .claimReward()
          .accounts({
            flashPool: flashPoolPda,
            userPrediction: userPredictionPda,
            vault: vaultPda,
            mint: USDC_MINT,
            userTokenAccount: userTokenAccount.address,
            user: userKey,
            treasury: treasury,
            payer: feePayer.publicKey,
            tokenProgram: TOKEN_2022_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          })
          .rpc();

        paidCount++;
        process.stdout.write(`\r  ✅ Paid ${paidCount}/${winners.length} winners`);
        return tx;
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        console.error(`\n  ❌ Claim failed: ${msg.slice(0, 120)}`);
        return null;
      }
    });

    await Promise.all(batchPromises);
    if (i + BATCH < winners.length) await sleep(200);
  }

  console.log("\n\n" + "═".repeat(55));
  console.log("✅ CRANK COMPLETE");
  console.log("═".repeat(55));
  console.log(`Winners paid: ${paidCount}/${winners.length}`);
  console.log(`Outcome:      $${(Number(outcome) / 100).toFixed(2)}`);
  console.log(`Median error: ±${medianError} buckets`);
}

main().catch(err => {
  console.error("Fatal error:", err);
  process.exit(1);
});
