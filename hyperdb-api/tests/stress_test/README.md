# Hyper DB Monte Carlo Stress Test

A comprehensive, long-running stress test for Hyper DB that simulates concurrent multi-user
workloads against a single `HyperProcess`. Uses Monte Carlo (stochastic) operation selection
with configurable resource limits, transport modes, and seed-based deterministic replay.

## Quick Start

```bash
# Default run (5 min, 3 databases, 9 user threads):
HYPERD_PATH=~/dev/bin/hyperd \
  cargo test -p hyperdb-api --test stress_test -- --ignored --nocapture

# High-load 2-minute run:
STRESS_DURATION=120 STRESS_DATABASES=5 STRESS_INSERTER_USERS=8 \
  STRESS_QUERY_USERS=6 STRESS_MIXED_USERS=4 STRESS_SEED=9999 \
  STRESS_THINK_MIN_MS=0 STRESS_THINK_MAX_MS=5 \
  STRESS_BATCH_MIN=1000 STRESS_BATCH_MAX=50000 \
  STRESS_OUTPUT_DIR=/tmp/stress_run \
  HYPERD_PATH=~/dev/bin/hyperd \
  cargo test -p hyperdb-api --test stress_test stress_test_tcp_hyperbinary -- --ignored --nocapture

# Replay a previous run:
STRESS_REPLAY_FILE=/tmp/stress_run/replay.json \
  STRESS_OUTPUT_DIR=/tmp/stress_replay \
  HYPERD_PATH=~/dev/bin/hyperd \
  cargo test -p hyperdb-api --test stress_test stress_test_replay -- --ignored --nocapture
```

All tests are `#[ignore]` so they never run during normal `cargo test`.

## Architecture

### How It Works

1. **HyperProcess startup** — A single `hyperd` instance is started with configurable
   resource logging (`log_resource_usage_interval`, `log_resource_usage_mode=2047`).

2. **Database setup** — `N` separate `.hyper` database files are created, each with
   2–5 tables of varying complexity (small/medium/wide schemas). Tables are seeded
   with 100 rows so queries have data from the start.

3. **User thread spawning** — Each simulated user runs in its own OS thread with a
   dedicated RNG seeded deterministically from the global seed. Users are divided
   into three classes with different operation probability distributions.

4. **Monte Carlo operation loop** — Each user thread repeatedly:
   - Rolls a random operation from its class's probability table
   - Checks disk budget (switches writes to reads if over limit)
   - Checks backpressure flag (pauses if resource thresholds exceeded)
   - Executes the operation against its assigned database
   - Sleeps for a random think-time
   - Records the outcome (latency, success/failure, rows affected)

5. **Resource monitoring** — A dedicated thread tails `hyperd.log`, parses
   `resource-metrics` JSON entries, and tracks memory, load, and CPU usage.
   When thresholds are exceeded, it sets a backpressure flag that user threads check.

6. **Completion** — After the configured duration (or on Hyper crash), all threads
   are joined and two output artifacts are written: `summary.json` and `replay.json`.

### User Classes

| Operation | Inserter (70% write) | Query (90% read) | Mixed (50/50) |
|-----------|---------------------|-------------------|---------------|
| Bulk Insert (COPY) | 0.70 | 0.05 | 0.35 |
| Single-row INSERT | 0.15 | 0.05 | 0.15 |
| Simple SELECT | 0.05 | 0.40 | 0.20 |
| Aggregate query | 0.03 | 0.25 | 0.15 |
| Complex JOIN query | 0.02 | 0.20 | 0.10 |
| Schema DDL | 0.05 | 0.05 | 0.05 |

### Table Schemas

Generated deterministically from per-database schema seeds:

- **Small** (3 cols): `id INT`, `name TEXT`, `value DOUBLE`
- **Medium** (8 cols): adds `created_at TIMESTAMP`, `category TEXT`, `quantity INT`, `price DOUBLE`, `description TEXT`
- **Wide** (20 cols): adds `is_active BOOL`, `rating SMALLINT`, `score DOUBLE`, `notes TEXT`, `updated_at TIMESTAMP`, `ref_code TEXT`, `weight DOUBLE`, `height DOUBLE`, `tag_a TEXT`, `tag_b TEXT`, `counter BIGINT`, `fraction DOUBLE`

### Transport Modes

| Test | Transport | Insert Method | Query Method |
|------|-----------|---------------|--------------|
| `stress_test_tcp_hyperbinary` | TCP | `Inserter` (HyperBinary COPY) | `Connection::execute_query` |
| `stress_test_tcp_arrow` | TCP | `ArrowInserter` (Arrow COPY) | `Connection::execute_query` |
| `stress_test_grpc` | TCP+gRPC | `Inserter` via TCP | `GrpcConnection` for reads |

## Configuration

All parameters are set via environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `STRESS_DURATION` | `300` | Simulation duration in seconds |
| `STRESS_DATABASES` | `3` | Number of `.hyper` database files |
| `STRESS_INSERTER_USERS` | `4` | Number of write-heavy user threads |
| `STRESS_QUERY_USERS` | `3` | Number of read-heavy user threads |
| `STRESS_MIXED_USERS` | `2` | Number of balanced user threads |
| `STRESS_SEED` | random | Global RNG seed for reproducibility |
| `STRESS_MAX_MEMORY_MB` | `4096` | Backpressure threshold for Hyper memory |
| `STRESS_MAX_CPU_PERCENT` | `90` | Backpressure threshold for CPU |
| `STRESS_MAX_DISK_MB` | `2048` | Disk space cap across all databases |
| `STRESS_THINK_MIN_MS` | `10` | Minimum think-time between operations |
| `STRESS_THINK_MAX_MS` | `200` | Maximum think-time between operations |
| `STRESS_BATCH_MIN` | `100` | Minimum rows per bulk insert |
| `STRESS_BATCH_MAX` | `10000` | Maximum rows per bulk insert |
| `STRESS_LOG_INTERVAL` | `5` | Hyper resource logging interval (seconds) |
| `STRESS_LOG_MODE` | `2047` | Hyper `log_resource_usage_mode` bitmask (2047 = All) |
| `STRESS_MEMORY_LIMIT` | `80%` | Hyper `memory_limit` setting |
| `STRESS_TRANSPORT` | `tcp` | Transport mode: `tcp` or `grpc` |
| `STRESS_DATA_FORMAT` | `hyperbinary` | Insert format: `hyperbinary` or `arrow` |
| `STRESS_OUTPUT_DIR` | temp dir | Output directory for results and `.hyper` files |
| `STRESS_REPLAY_FILE` | — | Path to `replay.json` for replay mode |
| `STRESS_QUERY_COMPLEXITY_MAX` | `3` | Max query complexity level |

## Output Artifacts

### `summary.json`

Complete run results including config snapshot and aggregate statistics:
- Operation counts by type (BulkInsert, SingleInsert, SimpleSelect, etc.)
- Throughput (ops/sec, insert-rows/sec)
- Latency percentiles (p50, p95, p99, max) overall and per-operation
- Peak resource usage (memory MB, CPU %, scheduler load)
- Disk space consumed
- Error details (up to 100 captured)
- Hyper crash detection

### `replay.json`

Compact seed-based replay file for deterministic reproduction:
- Global seed and full config snapshot
- Per-user seed assignments (user ID, class, seed, database index)
- Per-database schema seeds
- Stop reason (`DurationElapsed`, `CrashDetected`, `ResourceLimit`)

### Replay Mode

When `STRESS_REPLAY_FILE` is set, the simulator loads the config and all seeds from
the replay file (ignoring other env vars), spawns the same user threads with the same
per-user seeds, and runs with natural thread scheduling. Each user thread produces the
exact same sequence of operations because the per-thread RNG is seeded identically.
Thread interleaving varies between runs — this is intentional, as concurrent timing
is what triggers Hyper edge cases.

## Resource Monitoring

The monitor thread tails `hyperd.log` and parses JSON entries with `"k":"resource-metrics"`.

### Metrics Parsed

| Metric | JSON Path | Notes |
|--------|-----------|-------|
| Process memory (MB) | `v.memory.process_physical_memory_mb` | Direct f64 |
| Overall load | `v.load.overall_load` | 0.0–1.0+, >1.0 = overloaded |
| Scheduler load | `v.load.scheduler_load` | Scheduler-specific load |
| Memory load | `v.load.memory_load` | Memory pressure indicator |
| Scheduler threads | `v["scheduler-thread-count"].scheduler_thread_count` | Object with `active`/`inactive` |
| Waiting jobs | `v["scheduler-thread-count"].scheduler_waiting_jobs_count` | Direct i64 |
| Process CPU | `v.cpu.process_cpu_utilization` | Empty `{}` on macOS |
| System CPU | `v.cpu.system_cpu_utilization` | Empty `{}` on macOS |

### Hyper Settings Used

| Setting | Value | Purpose |
|---------|-------|---------|
| `log_resource_usage_interval` | Configurable (default 5s) | How often metrics are logged |
| `log_resource_usage_mode` | `2047` (All) | Bitmask: memory + trackers + scheduler + filesystem + load + CPU + cache + multiplexer + thread count + CPU time + memory buckets |
| `log_resource_usage_always` | `1` | Log even when idle |
| `memory_limit` | Configurable (default `80%`) | Hyper's global memory limit |

## Test Run Results

Results from actual runs on a Mac Studio (M2 Ultra, 192 GB RAM):

### Moderate Load (30s, 12 threads, seed=7777)

```
Duration:    30.5s
Operations:  25,185 (0 failures)
Throughput:  827 ops/sec, 297,808 insert-rows/sec
Latency:     p50=4.8ms  p95=45.4ms  p99=114.3ms  max=456.1ms
Peak memory: 2,344 MB
Peak load:   0.79
Disk used:   856 MB
```

### High Load (2 min, 18 threads, seed=9999)

```
Config:      5 DBs, 8 inserters, 6 query, 4 mixed
             think_time=0–5ms, batch=1k–50k rows
Duration:    121.0s
Operations:  16,275 (0 failures)
Throughput:  134.5 ops/sec, 195,132 insert-rows/sec
Latency:     p50=5.4ms  p95=126.6ms  p99=335.7ms  max=862.6ms
Peak memory: 4,348 MB
Peak load:   1.36 (overloaded — more work queued than cores available)
Disk used:   2,012 MB
```

### Replay of High Load Run

```
Duration:    120.8s
Operations:  13,720 (0 failures)
Throughput:  113.5 ops/sec, 191,867 insert-rows/sec
Latency:     p50=4.8ms  p95=126.9ms  p99=346.8ms  max=1,279.7ms
Peak memory: 4,287 MB
Peak load:   1.29
Disk used:   1,979 MB
```

**Observations:**
- Replay produces a similar workload profile — throughput, latency, and memory are
  in the same ballpark. Operation counts differ because thread scheduling varies.
- Hyper load >1.0 indicates the scheduler had more work queued than it could run
  concurrently. Despite this, zero failures occurred.
- Per-process CPU utilization is unavailable on macOS (`process_cpu_utilization` is
  `{}` in the log). The **load metric** is the best proxy for CPU pressure on macOS.
- Latency p99 increases significantly under high load (335ms vs 114ms), showing
  queuing effects.
- Hyper handled 18 concurrent threads hammering up to 50,000-row batch inserts with
  zero errors across all runs.

## File Structure

```
hyperdb-api/tests/
├── stress_test_main.rs          # Test entry points (#[ignore])
└── stress_test/
    ├── mod.rs                   # Module root
    ├── config.rs                # SimulationConfig + env var parsing
    ├── schema.rs                # Table schema generation (small/medium/wide)
    ├── user_profiles.rs         # User classes + probability distributions
    ├── workload.rs              # Operation implementations + data generation
    ├── resource_monitor.rs      # Hyper log tailer + backpressure enforcement
    ├── simulation.rs            # Monte Carlo engine (thread orchestration)
    ├── stats.rs                 # Stats aggregation + latency percentiles
    ├── replay.rs                # Replay/summary log read/write
    └── README.md                # This file
```

## Extending

### Adding New Operation Types

1. Add a variant to `OpKind` in `user_profiles.rs`
2. Add the probability weight to each user class's table in `OpDistribution::for_class`
3. Implement the operation function in `workload.rs`
4. Add the dispatch case in `execute_op`

### Adding New Table Schemas

1. Add a new function in `schema.rs` (e.g., `timeseries_table`)
2. Update `SchemaSize` enum and `build_table_def` to include it
3. Add the corresponding typed insertion logic in `add_row_direct` if new SQL types are used

### Custom Resource Thresholds

The backpressure system uses `AtomicBool` flags checked by user threads before each
operation. You can add new threshold checks in `resource_monitor.rs` by examining
additional fields from the `resource-metrics` log entries.
