/**
 * ⚡ FlashPool — Phase 3: 1,000-Bot Stress Test
 *
 * This script fires 1,000 `place_prediction` transactions at the
 * MagicBlock Magic Router RPC endpoint. Because the FlashPool PDA
 * is delegated to the Ephemeral Rollup:
 *
 *   1. The Magic Router detects the delegated account.
 *   2. It routes all 1,000 txs to the ER — NOT the Solana base layer.
 *   3. The ER processes them at 10–50ms speeds with ZERO gas fees.
 *   4. Each tx increments one histogram bucket in the FlashPool PDA.
 *
 * Expected outcome: FlashPool.histogram_buckets fills with a Gaussian
 * distribution centred around the current BTC/USD price.
 *
 * Run AFTER setup-pool.ts has successfully delegated the pool:
 *   npx ts-node scripts/bot-stress.ts
 *
 * Watch the frontend histogram fill in real-time while this runs!
 */

import * as anchor from "@coral-xyz/anchor";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
} from "@solana/web3.js";
import {
  TOKEN_2022_PROGRAM_ID,
  getOrCreateAssociatedTokenAccount,
  mintTo,
} from "@solana/spl-token";
import * as dotenv from "dotenv";
import * as fs from "fs";
import * as path from "path";

dotenv.config();

// ─── Config ────────────────────────────────────────────────────────────────────

/**
 * IMPORTANT: Use the Magic Router RPC — NOT the standard Devnet RPC.
 * The Magic Router detects delegated accounts and routes txs to the ER.
 * Using api.devnet.solana.com here would send txs to base layer and fail.
 */
const MAGIC_ROUTER_RPC = "https://devnet-rpc.magicblock.app";

/** Number of bot predictions to fire */
const BOT_COUNT = 1_000;

/**
 * Batch size for concurrent sends.
 * Higher = faster, but more likely to hit RPC rate limits.
 * 50 concurrent txs is a safe default for Devnet.
 */
const BATCH_SIZE = 50;

/**
 * Delay between batches in milliseconds.
 * 0 = fire as fast as possible.
 * 100 = 100ms between batches (gentler on the RPC).
 */
const BATCH_DELAY_MS = 100;

// ─── Seeds ─────────────────────────────────────────────────────────────────────

const SEED_FLASH_POOL = Buffer.from("flash_pool");
const SEED_USER_PREDICTION = Buffer.from("user_prediction");
const SEED_VAULT = Buffer.from("vault");

// ─── Price Math ────────────────────────────────────────────────────────────────

/**
 * Box-Muller transform — converts two uniform [0,1] randoms into one
 * standard normal random (same algorithm as the React frontend).
 */
function gaussianRandom(): number {
  const u1 = Math.random();
  const u2 = Math.random();
  return Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2);
}

/**
 * Maps a raw price to a histogram bucket index.
 * Must mirror the on-chain Rust logic exactly.
 */
function priceToBucketIndex(
  price: bigint,
  basePrice: bigint,
  precisionStep: bigint
): number {
  if (price < basePrice) return 0;
  const diff = price - basePrice;
  const idx = Number(diff / precisionStep);
  return Math.min(idx, 99); // cap at 99
}

// ─── Helpers ───────────────────────────────────────────────────────────────────

function loadKeypair(filePath: string): Keypair {
  const expanded = filePath.replace("~", process.env.HOME ?? "");
  const raw = fs.readFileSync(path.resolve(expanded), "utf-8");
  return Keypair.fromSecretKey(new Uint8Array(JSON.parse(raw)));
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// ─── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  console.log("🤖 FlashPool Phase 3 — 1,000 Bot Stress Test");
  console.log("─".repeat(55));
  console.log(`RPC:        ${MAGIC_ROUTER_RPC}`);
  console.log(`Bot count:  ${BOT_COUNT}`);
  console.log(`Batch size: ${BATCH_SIZE} concurrent txs`);
  console.log(`Batch delay: ${BATCH_DELAY_MS}ms`);
  console.log("");

  // Load admin / fee-payer wallet
  const feePayer = loadKeypair("~/.config/solana/id.json");
  console.log(`Fee payer: ${feePayer.publicKey.toBase58()}`);

  // Connect through the Magic Router
  const connection = new Connection(MAGIC_ROUTER_RPC, "confirmed");

  // Load IDL and program
  const idlPath = path.resolve(__dirname, "../target/idl/flash_pool.json");
  if (!fs.existsSync(idlPath)) {
    throw new Error("IDL not found. Run `anchor build` first.");
  }
  const idl = JSON.parse(fs.readFileSync(idlPath, "utf-8"));

  const deployKeyPath = path.resolve(__dirname, "../target/deploy/flash_pool-keypair.json");
  const deployKey = Keypair.fromSecretKey(
    new Uint8Array(JSON.parse(fs.readFileSync(deployKeyPath, "utf-8")))
  );
  const PROGRAM_ID = deployKey.publicKey;

  const provider = new anchor.AnchorProvider(
    connection,
    new anchor.Wallet(feePayer),
    { commitment: "confirmed", skipPreflight: true }
  );
  anchor.setProvider(provider);
  const program = new anchor.Program(idl, PROGRAM_ID, provider);

  // Read oracle feed from env or default to Pyth Lazer BTC/USD
  const oracleFeedStr =
    process.env.ORACLE_FEED ?? "71wtTRDY8Gxgw56bXFt2oc6qeAbTxzStdNiC425Z51sr";
  const oracleFeed = new PublicKey(oracleFeedStr);

  // Derive FlashPool and Vault PDAs
  const [flashPoolPda] = PublicKey.findProgramAddressSync(
    [SEED_FLASH_POOL, oracleFeed.toBuffer()],
    PROGRAM_ID
  );
  const [vaultPda] = PublicKey.findProgramAddressSync(
    [SEED_VAULT, flashPoolPda.toBuffer()],
    PROGRAM_ID
  );

  console.log(`\nFlashPool PDA: ${flashPoolPda.toBase58()}`);
  console.log(`Vault PDA:     ${vaultPda.toBase58()}`);

  // Fetch current pool state to get base_price and precision_step
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const poolAccount = await (program.account as any).flashPool.fetch(flashPoolPda);
  const basePrice: bigint = BigInt(poolAccount.basePrice.toString());
  const precisionStep: bigint = BigInt(poolAccount.precisionStep.toString());

  console.log(`\nPool state:`);
  console.log(`  base_price:      ${basePrice} ($${Number(basePrice) / 100})`);
  console.log(`  precision_step:  ${precisionStep} ($${Number(precisionStep) / 100})`);
  console.log(`  participants so far: ${poolAccount.totalParticipants}`);

  // USDC mint (same one used in setup-pool.ts)
  const USDC_MINT = new PublicKey("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");

  // Centre the Gaussian around the midpoint of the histogram
  // ($base_price + 50 * precision_step = the middle bucket)
  const centreBucket = 50;
  const centrePrice = basePrice + BigInt(centreBucket) * precisionStep;
  const sigmaBuckets = 15; // spread — standard deviation in buckets
  const sigmaPrice = BigInt(Math.round(sigmaBuckets)) * precisionStep;

  console.log(`\nGaussian params:`);
  console.log(`  centre: bucket ${centreBucket} ($${Number(centrePrice) / 100})`);
  console.log(`  sigma:  ${sigmaBuckets} buckets ($${Number(sigmaPrice) / 100})`);

  // ── Fire the bots ─────────────────────────────────────────────────────────

  console.log(`\n🚀 Firing ${BOT_COUNT} bots in batches of ${BATCH_SIZE}...\n`);

  let successCount = 0;
  let failCount = 0;
  const startTime = Date.now();
  const bucketHitCounts = new Array(100).fill(0);

  for (let batchStart = 0; batchStart < BOT_COUNT; batchStart += BATCH_SIZE) {
    const batchEnd = Math.min(batchStart + BATCH_SIZE, BOT_COUNT);
    const batchSize = batchEnd - batchStart;

    // Create one ephemeral keypair per bot in this batch.
    // Each bot needs a unique keypair so its UserPrediction PDA is unique.
    const bots = Array.from({ length: batchSize }, () => Keypair.generate());

    // For each bot, we need to:
    //   1. Fund it with SOL (for the UserPrediction PDA rent)
    //   2. Give it a USDC token account with 1 USDC (the entry fee)
    //   3. Send the place_prediction transaction
    //
    // In a real production crank, you'd use a single fee-payer wallet
    // and session keys. For the stress test, the fee-payer funds all bots.

    const batchPromises = bots.map(async (botKeypair, i) => {
      const botIdx = batchStart + i;

      try {
        // Generate a Gaussian price for this bot
        const gauss = gaussianRandom();
        const rawOffset = Number(sigmaPrice) * gauss;
        const botPrice = centrePrice + BigInt(Math.round(rawOffset));
        const bucketIndex = priceToBucketIndex(botPrice, basePrice, precisionStep);

        // The prediction_value we pass to the instruction must be
        // the exact bucket price (base + bucket_index * step), not the raw Gaussian price.
        // This ensures the on-chain bucket math maps correctly.
        const predictionValue = basePrice + BigInt(bucketIndex) * precisionStep;

        // Derive the UserPrediction PDA for this bot
        const [userPredictionPda] = PublicKey.findProgramAddressSync(
          [SEED_USER_PREDICTION, flashPoolPda.toBuffer(), botKeypair.publicKey.toBuffer()],
          PROGRAM_ID
        );

        // Create/get the bot's USDC ATA
        // In the real ER scenario, the fee-payer sponsors this.
        // For the stress test, we use the fee-payer's own USDC account instead
        // (simpler) and pass the fee-payer as the `user` but sign with the bot keypair.
        //
        // NOTE: In production, each user would have their own wallet and sign themselves.
        // The ER's gasless model means they don't pay SOL fees — but they still sign.

        // For the stress test, use the fee-payer's token account as the USDC source
        // (it must have at least BOT_COUNT USDC)
        const userTokenAccount = await getOrCreateAssociatedTokenAccount(
          connection,
          feePayer,
          USDC_MINT,
          feePayer.publicKey,
          false,
          "confirmed",
          {},
          TOKEN_2022_PROGRAM_ID
        );

        // Send the place_prediction transaction
        const tx = await program.methods
          .placePrediction(new anchor.BN(predictionValue.toString()))
          .accounts({
            flashPool: flashPoolPda,
            userPrediction: userPredictionPda,
            vault: vaultPda,
            mint: USDC_MINT,
            userTokenAccount: userTokenAccount.address,
            user: feePayer.publicKey, // fee-payer as the "user" for the stress test
            tokenProgram: TOKEN_2022_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          })
          .rpc();

        successCount++;
        bucketHitCounts[bucketIndex]++;

        if (botIdx % 100 === 0 || botIdx === BOT_COUNT - 1) {
          const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
          const tps = (successCount / parseFloat(elapsed)).toFixed(0);
          process.stdout.write(
            `\r   ✅ ${successCount}/${BOT_COUNT} sent (${tps} tx/s, ${elapsed}s elapsed)   `
          );
        }

        return { success: true, bucketIndex, tx };
      } catch (err: unknown) {
        failCount++;
        const msg = err instanceof Error ? err.message : String(err);
        if (process.env.VERBOSE === "1") {
          console.error(`\n   ❌ Bot ${botIdx} failed: ${msg.slice(0, 120)}`);
        }
        return { success: false, error: msg };
      }
    });

    await Promise.all(batchPromises);

    if (BATCH_DELAY_MS > 0 && batchEnd < BOT_COUNT) {
      await sleep(BATCH_DELAY_MS);
    }
  }

  // ── Results ───────────────────────────────────────────────────────────────

  const totalTime = ((Date.now() - startTime) / 1000).toFixed(2);
  const avgTps = (successCount / parseFloat(totalTime)).toFixed(1);

  console.log("\n\n" + "═".repeat(55));
  console.log("📊 STRESS TEST RESULTS");
  console.log("═".repeat(55));
  console.log(`✅ Successful txs: ${successCount}/${BOT_COUNT}`);
  console.log(`❌ Failed txs:     ${failCount}/${BOT_COUNT}`);
  console.log(`⏱  Total time:     ${totalTime}s`);
  console.log(`⚡ Average TPS:    ${avgTps} tx/s`);
  console.log("");

  // Print a mini ASCII histogram
  console.log("Bucket distribution (top 20 most-hit buckets):");
  const topBuckets = bucketHitCounts
    .map((count, idx) => ({ idx, count }))
    .sort((a, b) => b.count - a.count)
    .slice(0, 20);

  const maxCount = Math.max(...topBuckets.map(b => b.count));
  topBuckets.forEach(({ idx, count }) => {
    const bar = "█".repeat(Math.round((count / maxCount) * 30));
    const price = (Number(basePrice + BigInt(idx) * precisionStep) / 100).toFixed(2);
    console.log(`  Bucket ${String(idx).padStart(2)}: ${bar} ${count} bets ($${price})`);
  });

  console.log("\n🎉 Done! Check the frontend — histogram should be live.");
  console.log("   Run `resolve_market` to settle the pool.");
}

main().catch(err => {
  console.error("Fatal error:", err);
  process.exit(1);
});
