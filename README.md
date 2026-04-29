# MIDAS Backtesting Engine

> Deterministic Rust backtesting and validation engine for crypto strategy survivability analysis.

![Rust](https://img.shields.io/badge/Rust-2021_edition-orange)
![Crate](https://img.shields.io/badge/crate-midas--backtesting--engine-blue)
![License](https://img.shields.io/badge/License-MIT-purple)
![Build](https://img.shields.io/badge/build-cargo_test-brightgreen)

---

## What Is MIDAS?

MIDAS is a **library-first, deterministic backtesting and strategy validation engine** written in Rust. It answers one question: **does a strategy survive realistic historical conditions under explicit, reproducible assumptions?**

It is intentionally neutral:
- it is **not a trading bot**
- it does **not connect to exchanges**
- it does **not make trade recommendations**
- every output is **reproducible** — same inputs always produce the same report

MIDAS is designed to be called as a service, consumed by a CLI, or integrated as a library dependency. All outputs are stable, versioned JSON and CSV artifacts with explicit schema contracts, making them safe to parse downstream by other services, desktop apps, or AI agents.

---

## Relationship to Scenara

MIDAS was built alongside [Scenara](https://github.com/Noctilucenty/Scenara), a prediction market simulation platform. The two projects serve complementary roles:

| | MIDAS | Scenara |
|---|---|---|
| Language | Rust | Python (backend) + TypeScript (mobile) |
| Role | Strategy validation engine | Prediction market platform |
| Data flow | CSV / JSON historical data → deterministic report | Live CoinGecko prices → probabilistic market simulation |
| State | Stateless, library-first | Persistent database + live scheduler |
| Output | Versioned audit artifacts | Real-time leaderboard, portfolio, charts |
| Connection to exchanges | None (intentional) | None (simulation only) |

**Where they converge:** Scenara auto-generates crypto price-target markets from CoinGecko and auto-resolves them against live prices. MIDAS provides the backtesting infrastructure to validate whether those price-target assumptions are historically grounded — a natural upstream input to Scenara's market quality. The Parquet adapter scaffolding in MIDAS (`src/adapters/parquet.rs`) is the intended integration point for a shared data manager.

**Key differences from Scenara's approach:**
- MIDAS is purely deterministic; Scenara's probability engine uses Gaussian random walks
- MIDAS enforces explicit invariants and returns `InvariantViolation` errors on inconsistency; Scenara surfaces inconsistencies as UI states
- MIDAS has no user accounts, sessions, or real-time components
- MIDAS artifacts are designed for machine consumption; Scenara surfaces data through a REST API and mobile UI

---

## Architecture

```
src/
├── lib.rs                    # Crate root, re-exports public surface
├── main.rs                   # CLI entrypoint (midas-cli)
├── domain/
│   ├── config.rs             # BacktestConfig, ValidationConfig — typed request schemas
│   ├── errors.rs             # BacktestError, InvariantViolation — structured errors
│   ├── types.rs              # Candle, Event, MarketDataSet, trade primitives
│   └── mod.rs
├── engine/
│   ├── backtester.rs         # Core candle and event replay loop
│   ├── state.rs              # Account state, position tracking, equity curve
│   ├── invariants.rs         # Post-run consistency check set
│   ├── strategy.rs           # Strategy trait and built-in MA crossover strategy
│   └── mod.rs
├── execution/
│   ├── model.rs              # Fill simulation, liquidation, leg-level audit capture
│   └── mod.rs
├── metrics/
│   └── (portfolio + trade metric calculations)
├── validation/
│   └── mod.rs                # Split testing, robustness scoring, verdict reasons
├── reporting/
│   └── mod.rs                # JSON + CSV artifact generation, schema versioning
├── adapters/
│   ├── csv.rs                # CSV data source
│   ├── json.rs               # JSON data source
│   ├── parquet.rs            # Parquet schema scaffolding (read-ready contract)
│   ├── provider.rs           # Provider contract types + validation
│   └── mod.rs
├── api/
│   └── service.rs            # BacktestService — typed public entry point
└── bin/
    └── midas-bench.rs        # Lightweight performance benchmark harness
```

---

## Getting Started

### Prerequisites

- Rust 1.75+ (`rustup update stable`)
- Cargo (bundled with Rust)

### Build

```bash
git clone https://github.com/Noctilucenty/MIDAS.git
cd MIDAS
cargo build --release
```

### Run Tests

```bash
cargo test
```

### Run the CLI

```bash
# Candle-mode backtest
cargo run --bin midas-cli -- backtest \
  --config examples/configs/backtest_ma.json \
  --output examples/output/backtest

# Event-mode backtest (microstructure replay)
cargo run --bin midas-cli -- backtest \
  --config examples/configs/backtest_events.json \
  --output examples/output/event_backtest

# Strategy validation (walk-forward + robustness scoring)
cargo run --bin midas-cli -- validate \
  --config examples/configs/validation_ma.json \
  --output examples/output/validation
```

### Run the Benchmark

```bash
cargo run --bin midas-bench -- --candles 50000 --events 100000 --sweep-values 6
```

Reports candle replay throughput, event replay throughput, validation elapsed time, and parameter sweep run count.

---

## Execution Modes

### Candle Mode

Input: `MarketDataSet::Candles` — standard OHLCV bars.

- Funding applies per candle using `candle.funding_rate`
- Market orders execute on the next eligible bar after latency
- Limit orders evaluated against bar `open` / `high` / `low`
- Timeout handling is deterministic; fills cannot occur after timeout expiry
- Liquidation uses adverse intrabar path checks

### Event Mode

Input: `MarketDataSet::Events` — tick-level market events.

- Replay order: deterministic by `timestamp`, then `sequence`, then original index
- Snapshots update quote, trade, funding, and basic depth state
- Funding applies on explicit funding events
- Liquidation uses event-time mark data
- Forced flattening rejects cleanly when no executable price exists

Event mode is a microstructure-aware replay foundation, not a full exchange matching engine. Full L2 depth semantics and queue-position modeling are on the roadmap.

---

## Execution Audit

### Order Lifecycle (`order_audit_log.json`)

Each order produces exactly two entries:

1. `submitted` — when the order intent is queued
2. A terminal entry — `filled`, `cancelled`, `expired`, or `rejected`

Terminal entries include:
- `execution_leg_count`
- `position_before` / `position_after`
- `reason_code` (machine-readable)

### Execution Legs (`execution_legs.json`)

The trust-critical audit trail. Each fill leg is recorded separately with:

- Semantic role: `open`, `close`, or `liquidation`
- Side and quantity
- Requested and realized order kind
- Fee, spread, slippage, and funding attribution
- Timestamps and fill delay
- Position state before and after the leg
- Machine-readable `reason_code`

For flip orders (long→short, short→long), every close leg and reopen leg is captured individually, making the transition unambiguous for human review and downstream AI agents.

---

## Invariant Checks

Every successful backtest run passes an explicit consistency check set before the report is returned. Results are also exported to `consistency_report.json`.

| Check | Description |
|---|---|
| Equity reconciliation | Each equity point = `cash + unrealized_pnl` |
| Flat equity | Flat equity points carry no unrealized PnL |
| Drawdown | All drawdown points ≥ 0, reconcile with running peak |
| Trade count | `trade_count` matches trade log length |
| Ending equity | `ending_equity` matches last equity curve point |
| Net PnL | `net_pnl = ending_equity - starting_cash` |
| Cost totals | Fee, funding, spread, slippage totals reconcile with trades + open position carry |
| Leg matching | Execution legs match fills one-for-one |
| Close leg reconciliation | Closed trades match close/liquidation legs |
| Order progressions | Lifecycle state transitions are legal |
| Final position | Final position state matches final equity point |

Invariant failures return structured `InvariantViolation` errors instead of being silently swallowed.

---

## Validation

Validation answers whether a strategy generalizes, not just whether it fit historical data.

### Validation Report (`validation_report.json`)

Includes machine-readable verdict reason codes:

| Code | Meaning |
|---|---|
| `negative_out_of_sample_pnl` | OOS period lost money |
| `excessive_out_of_sample_drawdown` | OOS drawdown exceeded threshold |
| `stress_failures` | Strategy broke under stress conditions |
| `parameter_instability` | Results unstable across parameter variations |
| `walk_forward_instability` | Walk-forward windows disagree substantially |
| `non_deterministic_results` | Repeated runs produced different outputs |
| `insufficient_trades` | Too few trades to make statistical claims |
| `insufficient_validation_coverage` | Insufficient data coverage for validation |

### Validation Diagnostics (`validation_diagnostics.json`)

- `score_explanation`: breakdown of robustness score components
- `verdict_explanation`: human-readable explanations derived from reason codes

**Philosophy:** do not prove that a strategy looked profitable. Prove whether it survives realistic conditions.

---

## Execution Diagnostics (`execution_diagnostics.json`)

| Field | Description |
|---|---|
| `orders_submitted/filled/cancelled/expired/rejected` | Order outcome counts |
| `fill_mode_market/maker_limit/taker_limit` | Fill mode breakdown |
| `legs_open/close/liquidation` | Leg-type counts |
| `flip_order_count` | Long↔short flip orders |
| `liquidation_count` | Forced liquidations |
| `funding_applications` | Funding events applied |
| `avg_fill_delay_ms` | Average fill latency |
| `total_fee_paid` | Exchange fees |
| `total_spread_cost` | Spread cost |
| `total_slippage_cost` | Slippage |
| `total_funding_cost` | Funding carry cost |
| `total_liquidation_fees` | Liquidation penalty fees |
| `reason_counts` | Deterministic breakdown by reason code |

---

## Output Artifacts

### Backtest Run

| File | Description |
|---|---|
| `backtest_report.json` | Full backtest result with schema version |
| `metrics.json` | Complete portfolio and trade metrics |
| `metrics_summary.json` | Human-readable metrics digest |
| `run_manifest.json` | Reproducibility manifest (inputs, versions, hash) |
| `execution_diagnostics.json` | Execution breakdown (see above) |
| `replay_diagnostics.json` | Replay-mode diagnostics |
| `consistency_report.json` | Invariant check results |
| `order_audit_log.json` / `.csv` | Order lifecycle audit |
| `execution_legs.json` / `.csv` | Leg-level execution audit |
| `trade_log.csv` | One row per completed trade |
| `equity_curve.csv` | Time-series equity |
| `drawdown_curve.csv` | Time-series drawdown |

### Validation Run

| File | Description |
|---|---|
| `validation_report.json` | Validation verdict + reason codes |
| `validation_diagnostics.json` | Score breakdown + verdict explanation |
| `base_backtest/` | Full backtest artifact set for the base period |

All JSON artifacts carry `artifact_schema_version` and are covered by golden tests in `tests/golden_reports.rs`.

---

## Golden Tests

Golden tests verify artifact schema stability across code changes.

```bash
# Normal test run — fails if any artifact changes unexpectedly
cargo test

# Intentional schema update — regenerates golden fixtures
UPDATE_GOLDENS=1 cargo test --test golden_reports
```

Golden fixtures live in `tests/golden/`.

---

## Provider Contract

The engine consumes `MarketDataSet` (domain-native). The provider boundary in `src/adapters/provider.rs` defines:

| Type | Purpose |
|---|---|
| `MarketDataRequest` | Request for historical data (symbol, timeframe, range) |
| `HistoricalDataSource` | Trait for data providers |
| `DataSourceCapabilities` | What the source supports (candles, events, timeframes) |
| `DataSourceMetadata` | Provenance metadata |
| `LoadedMarketData` | Result of a provider load (`dataset + metadata`) |
| `FileDataSource` | CSV / JSON file-backed source |
| `InMemoryDataSource` | In-memory source for testing |
| `SourceSchema` | Field contract for Parquet and future sources |

Provider-side validation checks:
- Symbol and timeframe alignment with the request
- Candle monotonicity and validity
- Event monotonicity and same-timestamp sequence ordering
- Request/data mode compatibility
- Optional source schema compatibility

`BacktestService::run_backtest_with_data_source(...)` re-validates loaded data at the service boundary so the engine does not trust adapter implementations blindly.

---

## Parquet Readiness

The engine does not read Parquet directly (intentional — keeps storage concerns outside the validation engine). `src/adapters/parquet.rs` provides schema scaffolding.

**Required normalized Parquet fields:**

Candles: `timestamp`, `open`, `high`, `low`, `close`, `volume`

Events: `timestamp`, `sequence`, `kind`

**Recommended integration path:**

1. Read and normalize Parquet upstream (e.g. a shared Rust data manager)
2. Validate schema against the normalized contract in `parquet.rs`
3. Map records into `MarketDataSet`
4. Return `LoadedMarketData { dataset, metadata }`
5. Call `BacktestService::run_backtest_with_data_source(...)`

This keeps Parquet I/O and reconciliation concerns outside MIDAS while still enforcing a clear contract at the boundary.

---

## Safe Entry Points

Future integrations (including AI agents) should use the typed service boundary:

```rust
use midas_backtesting_engine::api::service::BacktestService;

// File-backed run
BacktestService::run_backtest_from_file_request(config, output_dir)
BacktestService::run_validation_from_file_request(config, output_dir)

// Provider-backed run (for Parquet / custom data sources)
BacktestService::run_backtest_with_data_source(config, data_source, output_dir)
```

Why these entry points:
- Typed inputs — no raw string passing
- Deterministic outputs — identical inputs produce identical reports
- Explicit assumptions — config is part of the audit trail
- Stable machine-readable artifacts — safe for downstream parsing
- Provider validation at the boundary — the engine does not trust adapters

---

## Example Config

`examples/configs/backtest_ma.json` — moving average crossover backtest:

```json
{
  "symbol": "BTCUSDT",
  "timeframe": "1h",
  "data_path": "examples/data/btcusdt_1h.csv",
  "strategy": {
    "type": "MovingAverageCrossover",
    "fast_period": 10,
    "slow_period": 30
  },
  "execution": {
    "initial_cash": 10000.0,
    "position_size_pct": 0.95,
    "fee_rate": 0.001,
    "slippage_rate": 0.0005,
    "spread_rate": 0.0002
  }
}
```

---

## Roadmap

- [ ] Full L2 order-book depth semantics in event mode
- [ ] Queue-position modeling for maker limit fills
- [ ] Multi-level partial fills
- [ ] Direct Parquet reading adapter (if operationally justified)
- [ ] Deeper performance profiling on large parameter sweep grids
- [ ] Rule-based strategies operating directly on event streams
- [ ] More built-in strategy types beyond MA crossover
- [ ] WebAssembly compilation target for browser-side validation

---

## What MIDAS Does Not Do

- Connect to any exchange or data feed
- Make trade recommendations
- Store user data or state
- Provide real-time signals
- Prove that a strategy will be profitable

MIDAS proves whether a strategy survived historical conditions under explicit assumptions. What you do with that information is entirely your responsibility.

---

## Contributing

Pull requests are welcome. Core invariants:

1. All new artifacts must carry `artifact_schema_version`
2. Schema changes must update golden fixtures (`UPDATE_GOLDENS=1 cargo test`)
3. Execution audit must remain lifecycle-complete (every order has a terminal entry)
4. The service boundary must re-validate provider data before passing to the engine

---

## License

MIT © MIDAS 2026
