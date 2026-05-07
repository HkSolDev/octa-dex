'use client';

import { useState, useEffect, useRef, useCallback } from 'react';
import {
  LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip,
  ResponsiveContainer, ReferenceLine, BarChart, Bar, Cell,
} from 'recharts';

// ─── Constants ────────────────────────────────────────────────────────────────
const SOL_FEED_ID = 'ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d';
const HISTOGRAM_BUCKETS = 100;
const PRECISION = 0.10; // $ per bucket
const ENTRY_FEE = 1;    // USDC

// ─── Types ────────────────────────────────────────────────────────────────────
interface PricePoint { time: string; price: number; }
interface BucketData  { index: number; label: string; count: number; isWinner: boolean; isUserBet: boolean; }

// ─── Helpers ──────────────────────────────────────────────────────────────────
function buildBuckets(base: number, buckets: number[], userBucket: number | null, winBucket: number | null, medianError: number | null): BucketData[] {
  return buckets.map((count, i) => ({
    index: i,
    label: `$${(base + i * PRECISION).toFixed(2)}`,
    count,
    isUserBet: i === userBucket,
    isWinner: winBucket !== null && medianError !== null && Math.abs(i - winBucket) <= medianError,
  }));
}

function priceToBucket(price: number, base: number): number {
  const idx = Math.round((price - base) / PRECISION);
  return Math.max(0, Math.min(HISTOGRAM_BUCKETS - 1, idx));
}

// ─── Main Page ────────────────────────────────────────────────────────────────
export default function FlashPoolPage() {
  const [currentPrice, setCurrentPrice]   = useState<number | null>(null);
  const [priceHistory, setPriceHistory]   = useState<PricePoint[]>([]);
  const [basePrice, setBasePrice]         = useState<number | null>(null);
  const [buckets, setBuckets]             = useState<number[]>(Array(HISTOGRAM_BUCKETS).fill(0));
  const [userBet, setUserBet]             = useState<number | null>(null);     // bucket index
  const [predictionPrice, setPredictionPrice] = useState<string>('');
  const [userBalance, setUserBalance]     = useState<number>(100);  // mock USDC
  const [totalPool, setTotalPool]         = useState<number>(0);
  const [participants, setParticipants]   = useState<number>(0);
  const [botRunning, setBotRunning]       = useState<boolean>(false);
  const [botProgress, setBotProgress]     = useState<number>(0);
  const [resolved, setResolved]           = useState<boolean>(false);
  const [winBucket, setWinBucket]         = useState<number | null>(null);
  const [medianError, setMedianError]     = useState<number | null>(null);
  const [fetchError, setFetchError]       = useState<string | null>(null);
  const [priceLoaded, setPriceLoaded]     = useState<boolean>(false);

  const botRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // ─── Pyth Price Feed ──────────────────────────────────────────────────────
  const fetchPrice = useCallback(async () => {
    try {
      const res = await fetch(
        `https://hermes.pyth.network/v2/updates/price/latest?ids[]=${SOL_FEED_ID}`,
        { signal: AbortSignal.timeout(5000) }
      );
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const json = await res.json();
      const feed   = json.parsed[0];
      const raw    = parseInt(feed.price.price);
      const exp    = feed.price.expo;          // e.g. -8
      const price  = raw * Math.pow(10, exp);  // e.g. 150.000000xx
      const rounded = Math.round(price * 100) / 100; // 2 dp

      setCurrentPrice(rounded);
      setFetchError(null);
      setPriceLoaded(true);

      if (basePrice === null) {
        // Anchor the histogram base 5 buckets below current price
        const base = Math.round((rounded - 5 * PRECISION) * 10) / 10;
        setBasePrice(base);
        setPredictionPrice(rounded.toFixed(2));
      }

      setPriceHistory(prev => {
        const now = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
        const next = [...prev, { time: now, price: rounded }];
        return next.length > 60 ? next.slice(-60) : next;
      });
    } catch (e: unknown) {
      if (e instanceof Error && e.name !== 'AbortError') {
        setFetchError('Pyth feed unreachable — using mock data');
        // Fallback mock price
        const mock = 150 + (Math.random() - 0.5) * 2;
        const rounded = Math.round(mock * 100) / 100;
        setCurrentPrice(rounded);
        if (basePrice === null) {
          setBasePrice(Math.round((rounded - 5 * PRECISION) * 10) / 10);
          setPredictionPrice(rounded.toFixed(2));
        }
        setPriceLoaded(true);
        setPriceHistory(prev => {
          const now = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
          const next = [...prev, { time: now, price: rounded }];
          return next.length > 60 ? next.slice(-60) : next;
        });
      }
    }
  }, [basePrice]);

  useEffect(() => {
    fetchPrice();
    const interval = setInterval(fetchPrice, 2000);
    return () => clearInterval(interval);
  }, [fetchPrice]);

  // ─── Place User Bet ───────────────────────────────────────────────────────
  const placeBet = useCallback(() => {
    if (basePrice === null || userBet !== null || resolved) return;
    const price = parseFloat(predictionPrice);
    if (isNaN(price)) return;
    if (userBalance < ENTRY_FEE) { alert('Insufficient USDC balance'); return; }
    const idx = priceToBucket(price, basePrice);
    setBuckets(prev => { const n = [...prev]; n[idx]++; return n; });
    setUserBet(idx);
    setUserBalance(b => b - ENTRY_FEE);
    setTotalPool(p => p + ENTRY_FEE);
    setParticipants(c => c + 1);
  }, [basePrice, userBet, resolved, predictionPrice, userBalance]);

  // ─── Run Bot (1000 Bets) ──────────────────────────────────────────────────
  const runBot = useCallback(() => {
    if (botRunning || resolved || basePrice === null) return;
    setBotRunning(true);
    setBotProgress(0);
    let placed = 0;
    const TOTAL = 1000;

    // Gaussian distribution centred around current price
    const centre = currentPrice ?? (basePrice + 5 * PRECISION);
    const sigma = 2.5 * PRECISION;  // spread ≈ 5 buckets

    botRef.current = setInterval(() => {
      const BATCH = 20; // place 20 bets per tick (50ms) → 50 ticks total
      setBuckets(prev => {
        const n = [...prev];
        for (let k = 0; k < BATCH && placed < TOTAL; k++, placed++) {
          // Box-Muller transform for Gaussian random
          const u1 = Math.random(), u2 = Math.random();
          const gauss = Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2);
          const botPrice = centre + gauss * sigma;
          const idx = priceToBucket(botPrice, basePrice as number);
          n[idx]++;
        }
        return n;
      });
      setTotalPool(p => p + BATCH);
      setParticipants(c => c + BATCH);
      setBotProgress(placed);

      if (placed >= TOTAL) {
        clearInterval(botRef.current!);
        setBotRunning(false);
        setBotProgress(TOTAL);
      }
    }, 50);
  }, [botRunning, resolved, basePrice, currentPrice]);

  // ─── Resolve Market ───────────────────────────────────────────────────────
  const resolveMarket = useCallback(() => {
    if (currentPrice === null || basePrice === null || resolved) return;
    const winIdx = priceToBucket(currentPrice, basePrice);
    // Walk outward to find median error
    const target = Math.max(1, Math.floor(participants / 2));
    let acc = 0, err = 0;
    for (let d = 0; d < HISTOGRAM_BUCKETS; d++) {
      if (d === 0) acc += buckets[winIdx];
      else {
        const l = winIdx - d; if (l >= 0)                    acc += buckets[l];
        const r = winIdx + d; if (r < HISTOGRAM_BUCKETS)     acc += buckets[r];
      }
      err = d;
      if (acc >= target) break;
    }
    setWinBucket(winIdx);
    setMedianError(err);
    setResolved(true);
  }, [currentPrice, basePrice, resolved, participants, buckets]);

  // ─── Derived Data ─────────────────────────────────────────────────────────
  const bucketData = basePrice !== null
    ? buildBuckets(basePrice, buckets, userBet, winBucket, medianError)
    : [];

  const priceMin = priceHistory.length > 0 ? Math.min(...priceHistory.map(p => p.price)) : 0;
  const priceMax = priceHistory.length > 0 ? Math.max(...priceHistory.map(p => p.price)) : 0;
  const pricePad = Math.max((priceMax - priceMin) * 0.3, 0.05);

  const userWon = resolved && userBet !== null && winBucket !== null && medianError !== null
    && Math.abs(userBet - winBucket) <= medianError;
  const winners = resolved && winBucket !== null && medianError !== null
    ? buckets.slice(winBucket - medianError, winBucket + medianError + 1).reduce((s, v) => s + v, 0) : 0;

  // ─── Render ───────────────────────────────────────────────────────────────
  return (
    <main className="root">
      {/* Header */}
      <header className="header">
        <div className="logo">
          <span className="logo-icon">⚡</span>
          <span className="logo-text">FlashPool</span>
          <span className="logo-sub">SOL/USD Prediction Market</span>
        </div>
        <div className="header-stats">
          <div className="stat-pill">
            <span className="stat-label">BALANCE</span>
            <span className="stat-value green">${userBalance.toFixed(2)} USDC</span>
          </div>
          <div className="stat-pill">
            <span className="stat-label">POOL</span>
            <span className="stat-value cyan">${totalPool.toLocaleString()} USDC</span>
          </div>
          <div className="stat-pill">
            <span className="stat-label">PLAYERS</span>
            <span className="stat-value purple">{participants.toLocaleString()}</span>
          </div>
        </div>
      </header>

      <div className="grid">
        {/* ── Left: Live Price Chart ────────────────────────────────────────── */}
        <section className="card chart-card">
          <div className="card-header">
            <h2 className="card-title">
              <span className="dot green-dot" />
              Live SOL/USD
              {fetchError && <span className="mock-badge">MOCK</span>}
            </h2>
            <div className="price-display">
              <span className="current-price">
                ${currentPrice !== null ? currentPrice.toFixed(4) : '---'}
              </span>
              {priceHistory.length > 1 && (
                <span className={`price-delta ${priceHistory[priceHistory.length - 1].price >= priceHistory[priceHistory.length - 2].price ? 'up' : 'down'}`}>
                  {priceHistory[priceHistory.length - 1].price >= priceHistory[priceHistory.length - 2].price ? '▲' : '▼'}
                  {Math.abs(priceHistory[priceHistory.length - 1].price - priceHistory[priceHistory.length - 2].price).toFixed(4)}
                </span>
              )}
            </div>
          </div>

          <div className="chart-wrapper">
            {priceHistory.length > 1 ? (
              <ResponsiveContainer width="100%" height="100%">
                <LineChart data={priceHistory} margin={{ top: 4, right: 8, left: 0, bottom: 0 }}>
                  <defs>
                    <linearGradient id="priceGrad" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="5%"  stopColor="#00ffcc" stopOpacity={0.2} />
                      <stop offset="95%" stopColor="#00ffcc" stopOpacity={0} />
                    </linearGradient>
                  </defs>
                  <CartesianGrid strokeDasharray="3 3" stroke="rgba(255,255,255,0.05)" />
                  <XAxis dataKey="time" tick={{ fill: '#888', fontSize: 10 }} interval="preserveStartEnd" />
                  <YAxis domain={[priceMin - pricePad, priceMax + pricePad]} tick={{ fill: '#888', fontSize: 10 }} width={60} tickFormatter={v => `$${v.toFixed(2)}`} />
                  <Tooltip
                    contentStyle={{ background: '#0d1117', border: '1px solid #00ffcc44', borderRadius: '8px', color: '#fff' }}
                    formatter={(v: number) => [`$${v.toFixed(4)}`, 'SOL/USD']}
                  />
                  {userBet !== null && basePrice !== null && (
                    <ReferenceLine y={basePrice + userBet * PRECISION} stroke="#a855f7" strokeDasharray="5 3" label={{ value: 'Your Bet', fill: '#a855f7', fontSize: 11 }} />
                  )}
                  <Line type="monotone" dataKey="price" stroke="#00ffcc" strokeWidth={2} dot={false} activeDot={{ r: 4, fill: '#00ffcc' }} fill="url(#priceGrad)" />
                </LineChart>
              </ResponsiveContainer>
            ) : (
              <div className="loading-state">
                <div className="spinner" />
                <span>Fetching Pyth oracle data…</span>
              </div>
            )}
          </div>
        </section>

        {/* ── Right: Controls ───────────────────────────────────────────────── */}
        <section className="card controls-card">
          <h2 className="card-title"><span className="dot purple-dot" />Place Prediction</h2>

          <div className="control-group">
            <label className="ctrl-label">Your Price Prediction (USD)</label>
            <div className="input-row">
              <span className="input-prefix">$</span>
              <input
                className="price-input"
                type="number"
                step="0.01"
                value={predictionPrice}
                onChange={e => setPredictionPrice(e.target.value)}
                disabled={userBet !== null || resolved}
                placeholder={currentPrice?.toFixed(2) ?? '0.00'}
              />
            </div>
            {basePrice !== null && predictionPrice && !isNaN(parseFloat(predictionPrice)) && (
              <p className="hint">
                → Bucket #{priceToBucket(parseFloat(predictionPrice), basePrice)} 
                (${(basePrice + priceToBucket(parseFloat(predictionPrice), basePrice) * PRECISION).toFixed(2)} range)
              </p>
            )}
          </div>

          <button
            className={`btn btn-primary ${(userBet !== null || resolved || !priceLoaded) ? 'btn-disabled' : ''}`}
            onClick={placeBet}
            disabled={userBet !== null || resolved || !priceLoaded}
          >
            {userBet !== null ? '✓ Bet Placed' : `Place Bet ($${ENTRY_FEE} USDC)`}
          </button>

          <div className="divider" />

          <h2 className="card-title"><span className="dot cyan-dot" />Bot Simulation</h2>
          <p className="hint-text">Fire 1,000 Gaussian-distributed bets centred on current price.</p>

          <button
            className={`btn btn-bot ${(botRunning || resolved || !priceLoaded) ? 'btn-disabled' : ''}`}
            onClick={runBot}
            disabled={botRunning || resolved || !priceLoaded}
          >
            {botRunning ? `🤖 Running… ${botProgress}/1000` : '🤖 Run 1,000 Bot Bets'}
          </button>

          {botRunning && (
            <div className="progress-bar">
              <div className="progress-fill" style={{ width: `${(botProgress / 1000) * 100}%` }} />
            </div>
          )}

          <div className="divider" />

          <h2 className="card-title"><span className="dot red-dot" />Resolve Market</h2>
          <p className="hint-text">Settle the round using the current live Pyth price as the oracle outcome.</p>
          <button
            className={`btn btn-resolve ${(resolved || participants === 0 || !priceLoaded) ? 'btn-disabled' : ''}`}
            onClick={resolveMarket}
            disabled={resolved || participants === 0 || !priceLoaded}
          >
            {resolved ? '✓ Market Resolved' : '⚡ Resolve Now'}
          </button>

          {resolved && currentPrice !== null && (
            <div className={`result-box ${userWon ? 'result-win' : 'result-default'}`}>
              <p className="result-title">{userWon ? '🏆 You Won!' : '📊 Market Settled'}</p>
              <p className="result-stat">Oracle Price: <strong>${currentPrice.toFixed(4)}</strong></p>
              <p className="result-stat">Median Error: <strong>±{medianError} buckets</strong></p>
              <p className="result-stat">Winners: <strong>{winners} players</strong></p>
              {winners > 0 && <p className="result-stat">Payout/Winner: <strong>${(totalPool / winners).toFixed(2)} USDC</strong></p>}
            </div>
          )}
        </section>

        {/* ── Bottom: Histogram ─────────────────────────────────────────────── */}
        <section className="card histogram-card">
          <div className="card-header">
            <h2 className="card-title">
              <span className="dot cyan-dot" />
              Prediction Histogram
              <span className="bucket-count">{participants.toLocaleString()} bets across 100 buckets</span>
            </h2>
          </div>

          <div className="histogram-wrapper">
            {bucketData.length > 0 ? (
              <ResponsiveContainer width="100%" height="100%">
                <BarChart data={bucketData} margin={{ top: 4, right: 8, left: 0, bottom: 0 }} barCategoryGap="2%">
                  <CartesianGrid strokeDasharray="3 3" stroke="rgba(255,255,255,0.04)" vertical={false} />
                  <XAxis dataKey="index" tick={false} />
                  <YAxis tick={{ fill: '#888', fontSize: 10 }} width={32} />
                  <Tooltip
                    contentStyle={{ background: '#0d1117', border: '1px solid #00ffcc44', borderRadius: '8px', color: '#fff', fontSize: 12 }}
                    formatter={(v: number, _: string, props) => [
                      `${v} bets`,
                      props.payload.label,
                    ]}
                  />
                  <Bar dataKey="count" maxBarSize={12} radius={[2, 2, 0, 0]}>
                    {bucketData.map(entry => (
                      <Cell
                        key={`cell-${entry.index}`}
                        fill={
                          entry.isUserBet  ? '#a855f7' :
                          entry.isWinner   ? '#22c55e' :
                          winBucket !== null && entry.index === winBucket ? '#f59e0b' :
                          '#0ea5e9'
                        }
                        opacity={entry.isWinner || entry.isUserBet ? 1 : 0.65}
                      />
                    ))}
                  </Bar>
                </BarChart>
              </ResponsiveContainer>
            ) : (
              <div className="loading-state"><span>Waiting for price data…</span></div>
            )}
          </div>

          <div className="legend">
            <div className="legend-item"><span className="legend-dot" style={{ background: '#0ea5e9' }} />Bot Bets</div>
            <div className="legend-item"><span className="legend-dot" style={{ background: '#a855f7' }} />Your Bet</div>
            <div className="legend-item"><span className="legend-dot" style={{ background: '#22c55e' }} />Winner Zone</div>
            <div className="legend-item"><span className="legend-dot" style={{ background: '#f59e0b' }} />Oracle Price</div>
          </div>
        </section>
      </div>
    </main>
  );
}
