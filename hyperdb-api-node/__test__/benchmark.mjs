/**
 * Performance benchmark for hyperdb-api-node.
 *
 * Uses the same schema as the Rust API benchmark (hyperdb-api/benches/benchmark.rs):
 *   measurements(id INT NOT NULL, sensor_id INT, value DOUBLE, timestamp BIGINT)
 *
 * Benchmarks:
 *   1. Bulk insert via RowInserter (COPY protocol, row-per-row)
 *   1b. Bulk insert via ArrowInserter (COPY protocol, Arrow IPC batch)
 *   2. Full table scan (executeQuery - eager)
 *   3. Full table scan (executeQueryStream - streaming)
 *   4. Filtered query (WHERE sensor_id = 5)
 *   5. Server-side aggregation (GROUP BY sensor_id)
 *
 * Run with:
 *   HYPERD_PATH=/path/to/hyperd node __test__/benchmark.mjs [ROW_COUNT]
 *
 * Examples:
 *   node __test__/benchmark.mjs              # Default 1M rows
 *   node __test__/benchmark.mjs 10000000     # 10M rows
 */

import { createRequire } from 'module';
import { mkdirSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';
const require = createRequire(import.meta.url);

const __dirname = dirname(fileURLToPath(import.meta.url));
const TEST_DIR = join(__dirname, '..', 'test_results');
mkdirSync(TEST_DIR, { recursive: true });

const {
  HyperProcess,
  Connection,
  ConnectionBuilder,
  CreateMode,
  Catalog,
  TableDefinition,
  SqlType,
  RowInserter,
  ArrowInserter,
} = require('../index.js');

import { tableFromIPC, tableFromArrays, tableToIPC } from 'apache-arrow';

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

const DEFAULT_ROW_COUNT = 1_000_000;
const ROW_COUNT = parseInt(process.argv[2], 10) || DEFAULT_ROW_COUNT;

// Bytes per row: id(4) + sensor_id(4) + value(8) + timestamp(8) = 24
const BYTES_PER_ROW = 24;

// Batch size for addRows (larger = fewer JS→Rust boundary crossings)
const INSERT_BATCH_SIZE = 50_000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatCount(n) {
  if (n >= 1e9) return (n / 1e9).toFixed(1) + 'B';
  if (n >= 1e6) return (n / 1e6).toFixed(1) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(1) + 'K';
  return String(n);
}

function formatSize(bytes) {
  if (bytes >= 1e9) return (bytes / 1e9).toFixed(2) + ' GB';
  if (bytes >= 1e6) return (bytes / 1e6).toFixed(2) + ' MB';
  if (bytes >= 1e3) return (bytes / 1e3).toFixed(2) + ' KB';
  return bytes + ' B';
}

function mbPerSec(bytes, secs) {
  return secs > 0 ? bytes / secs / (1024 * 1024) : 0;
}

function printHeader(title) {
  console.log();
  console.log('='.repeat(78));
  console.log(`  ${title}`);
  console.log('='.repeat(78));
  console.log();
}

function pad(str, len, right = false) {
  const s = String(str);
  return right ? s.padEnd(len) : s.padStart(len);
}

function printResult(name, rowCount, elapsedMs) {
  const secs = elapsedMs / 1000;
  const rowsPerSec = rowCount / secs;
  const totalBytes = rowCount * BYTES_PER_ROW;
  const throughput = mbPerSec(totalBytes, secs);
  console.log(
    `  ${pad(name, 25, true)} ${pad(formatCount(rowCount), 10)} rows  ${pad(secs.toFixed(3), 8)} s  ${pad(formatCount(Math.round(rowsPerSec)), 12)} rows/sec  ${pad(throughput.toFixed(2), 8)} MB/s`
  );
  return { name, rowCount, elapsedMs, secs, rowsPerSec, throughput };
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

/**
 * Benchmark 1: Bulk insert via RowInserter (COPY protocol, row-per-row).
 *
 * Uses addRows() in batches to minimize JS→Rust boundary crossings.
 */
async function benchInsert(conn, tableDef, rowCount) {
  const inserter = new RowInserter(conn, tableDef);
  const batchSize = Math.min(INSERT_BATCH_SIZE, rowCount);

  const start = performance.now();

  let remaining = rowCount;
  let id = 0;

  while (remaining > 0) {
    const count = Math.min(batchSize, remaining);
    const batch = new Array(count);

    for (let i = 0; i < count; i++) {
      const sensorId = id % 10;
      const value = id * 0.1;
      const timestamp = 1700000000000 + id * 1000;
      batch[i] = [id, sensorId, value, timestamp];
      id++;
    }

    inserter.addRows(batch);
    remaining -= count;
  }

  await inserter.execute();
  const elapsed = performance.now() - start;

  return printResult('Insert (COPY, row API)', rowCount, elapsed);
}

/**
 * Benchmark 1b: Bulk insert via ArrowInserter.
 *
 * Build the whole dataset as an Arrow Table in JS, serialize to IPC once,
 * ship to hyperd in one insertRaw call. Zero per-row JS↔Rust conversion —
 * this is the path that closes most of the Rust↔Node throughput gap.
 */
async function benchInsertArrow(conn, tableDef, rowCount) {
  // Ship rows in Arrow IPC batches below hyperd's 150 MB packet limit.
  // At 24 bytes/row (4+4+8+8 bytes) a batch of 2.5M rows sits well
  // under the cap; keep 1M for headroom.
  const BATCH_ROWS = 1_000_000;

  const start = performance.now();
  const inserter = ArrowInserter.create(conn, tableDef);

  for (let offset = 0; offset < rowCount; offset += BATCH_ROWS) {
    const n = Math.min(BATCH_ROWS, rowCount - offset);
    const ids = new Int32Array(n);
    const sensorIds = new Int32Array(n);
    const values = new Float64Array(n);
    const timestamps = new BigInt64Array(n);
    for (let i = 0; i < n; i++) {
      const id = offset + i;
      ids[i] = id;
      sensorIds[i] = id % 10;
      values[i] = id * 0.1;
      timestamps[i] = BigInt(1700000000000 + id * 1000);
    }

    const batch = tableFromArrays({
      id: ids,
      sensor_id: sensorIds,
      value: values,
      timestamp: timestamps,
    });
    await inserter.insertRaw(Buffer.from(tableToIPC(batch, 'stream')));
  }
  await inserter.execute();
  const elapsed = performance.now() - start;

  return printResult('Insert (COPY, Arrow IPC)', rowCount, elapsed);
}

/**
 * Benchmark 2: Full table scan via executeQuery (all rows in memory).
 */
async function benchFullScanEager(conn, rowCount) {
  const start = performance.now();
  const rows = await conn.executeQuery('SELECT * FROM measurements ORDER BY id');

  let sumId = 0;
  let count = 0;
  for (const row of rows) {
    sumId += row.getInt32(0) ?? 0;
    count++;
  }

  const elapsed = performance.now() - start;
  const result = printResult('Full Scan (eager)', count, elapsed);
  console.log(`    checksum: sumId=${sumId}, rows=${count}`);
  return result;
}

/**
 * Benchmark 3: Full table scan via executeQueryStream (streaming, constant memory).
 */
async function benchFullScanStream(conn, rowCount) {
  const start = performance.now();
  const stream = conn.executeQueryStream('SELECT * FROM measurements ORDER BY id');

  let sumId = 0;
  let count = 0;
  for await (const row of stream) {
    sumId += row.getInt32(0) ?? 0;
    count++;
  }

  const elapsed = performance.now() - start;
  const result = printResult('Full Scan (stream)', count, elapsed);
  console.log(`    checksum: sumId=${sumId}, rows=${count}`);
  return result;
}

/**
 * Benchmark 4: Filtered query (WHERE sensor_id = 5).
 */
async function benchFilteredQuery(conn, rowCount) {
  const start = performance.now();
  const stream = conn.executeQueryStream(
    'SELECT * FROM measurements WHERE sensor_id = 5'
  );

  let count = 0;
  let sumValue = 0;
  for await (const row of stream) {
    sumValue += row.getFloat64(2) ?? 0;
    count++;
  }

  const elapsed = performance.now() - start;
  const result = printResult('Filtered (sensor=5)', count, elapsed);
  console.log(`    checksum: sumValue=${sumValue.toFixed(1)}, rows=${count}`);
  return result;
}

/**
 * Benchmark 5: Server-side aggregation (GROUP BY sensor_id).
 */
async function benchAggregation(conn, rowCount) {
  const start = performance.now();
  const rows = await conn.executeQuery(
    'SELECT sensor_id, AVG(value), COUNT(*) FROM measurements GROUP BY sensor_id ORDER BY sensor_id'
  );

  let totalCount = 0;
  let groups = 0;
  for (const row of rows) {
    totalCount += row.getInt32(2) ?? 0;
    groups++;
  }

  const elapsed = performance.now() - start;
  const result = printResult('Aggregation (GROUP BY)', totalCount, elapsed);
  console.log(`    groups=${groups}, totalCount=${totalCount}`);
  return result;
}

/**
 * Benchmark 6: Chunk-level streaming (nextChunk instead of row-by-row).
 */
async function benchFullScanChunked(conn, rowCount) {
  const start = performance.now();
  const stream = conn.executeQueryStream('SELECT * FROM measurements ORDER BY id');

  let sumId = 0;
  let count = 0;
  let chunk;
  while ((chunk = await stream.nextChunk()) !== null) {
    for (const row of chunk) {
      sumId += row.getInt32(0) ?? 0;
      count++;
    }
  }

  const elapsed = performance.now() - start;
  const result = printResult('Full Scan (chunked)', count, elapsed);
  console.log(`    checksum: sumId=${sumId}, rows=${count}`);
  return result;
}

/**
 * Benchmark 7: Columnar full scan via executeQueryColumnar.
 */
async function benchColumnarFullScan(conn, rowCount) {
  const start = performance.now();
  const stream = conn.executeQueryColumnar('SELECT * FROM measurements ORDER BY id');

  let sumId = 0;
  let count = 0;
  let chunk;
  while ((chunk = await stream.nextChunk()) !== null) {
    const ids = chunk.getInt32Column(0);
    count += chunk.rowCount;
    for (let i = 0; i < ids.length; i++) {
      sumId += ids[i];
    }
  }

  const elapsed = performance.now() - start;
  const result = printResult('Columnar Full Scan', count, elapsed);
  console.log(`    checksum: sumId=${sumId}, rows=${count}`);
  return result;
}

/**
 * Benchmark 8: Columnar filtered query.
 */
async function benchColumnarFiltered(conn, rowCount) {
  const start = performance.now();
  const stream = conn.executeQueryColumnar(
    'SELECT * FROM measurements WHERE sensor_id = 5'
  );

  let count = 0;
  let sumValue = 0;
  let chunk;
  while ((chunk = await stream.nextChunk()) !== null) {
    const values = chunk.getFloat64Column(2);
    count += chunk.rowCount;
    for (let i = 0; i < values.length; i++) {
      sumValue += values[i];
    }
  }

  const elapsed = performance.now() - start;
  const result = printResult('Columnar Filtered', count, elapsed);
  console.log(`    checksum: sumValue=${sumValue.toFixed(1)}, rows=${count}`);
  return result;
}

/**
 * Benchmark 9: Columnar insert via addColumnar.
 */
async function benchColumnarInsert(conn, tableDef, rowCount) {
  // Drop and recreate
  await conn.executeCommand('DROP TABLE IF EXISTS measurements_col');
  const colTableDef = new TableDefinition('measurements_col');
  colTableDef.addColumn('id', SqlType.int(), false);
  colTableDef.addColumn('sensor_id', SqlType.int(), true);
  colTableDef.addColumn('value', SqlType.double(), true);
  colTableDef.addColumn('timestamp', SqlType.bigInt(), true);
  const catalog = new Catalog(conn);
  await catalog.createTable(colTableDef);

  const inserter = new RowInserter(conn, colTableDef);
  const batchSize = Math.min(INSERT_BATCH_SIZE, rowCount);

  const start = performance.now();

  let remaining = rowCount;
  let id = 0;

  while (remaining > 0) {
    const count = Math.min(batchSize, remaining);
    const ids = new Array(count);
    const sensorIds = new Array(count);
    const values = new Array(count);
    const timestamps = new Array(count);

    for (let i = 0; i < count; i++) {
      ids[i] = id;
      sensorIds[i] = id % 10;
      values[i] = id * 0.1;
      timestamps[i] = 1700000000000 + id * 1000;
      id++;
    }

    inserter.addColumnar(
      { 0: ids, 1: sensorIds },
      { 2: values },
      { 3: timestamps },
      count
    );
    remaining -= count;
  }

  await inserter.execute();
  const elapsed = performance.now() - start;

  return printResult('Columnar Insert', rowCount, elapsed);
}

/**
 * Benchmark: Arrow IPC full scan — executeQueryToArrow + tableFromIPC.
 */
async function benchArrowFullScan(conn, rowCount) {
  const start = performance.now();

  const buf = await conn.executeQueryToArrow('SELECT * FROM measurements ORDER BY id');
  const table = tableFromIPC(buf);

  let sumId = 0;
  const ids = table.getChild('id').toArray();
  for (let i = 0; i < ids.length; i++) {
    sumId += ids[i];
  }

  const elapsed = performance.now() - start;
  const result = printResult('Arrow Full Scan', table.numRows, elapsed);
  console.log(`    checksum: sumId=${sumId}, rows=${table.numRows}`);
  return result;
}

/**
 * Benchmark: Arrow IPC filtered query.
 */
async function benchArrowFiltered(conn, rowCount) {
  const start = performance.now();

  const buf = await conn.executeQueryToArrow(
    'SELECT * FROM measurements WHERE sensor_id = 5'
  );
  const table = tableFromIPC(buf);

  let sumValue = 0;
  const values = table.getChild('value').toArray();
  for (let i = 0; i < values.length; i++) {
    sumValue += values[i];
  }

  const elapsed = performance.now() - start;
  const result = printResult('Arrow Filtered', table.numRows, elapsed);
  console.log(`    checksum: sumValue=${sumValue.toFixed(1)}, rows=${table.numRows}`);
  return result;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main() {
  printHeader('hyperdb-api-node Performance Benchmark');

  console.log('Configuration:');
  console.log(`  Row count:       ${ROW_COUNT.toLocaleString()} (${formatCount(ROW_COUNT)})`);
  console.log(`  Insert batch:    ${INSERT_BATCH_SIZE.toLocaleString()}`);
  console.log(`  Bytes per row:   ${BYTES_PER_ROW}`);
  console.log(`  Total data:      ${formatSize(ROW_COUNT * BYTES_PER_ROW)}`);

  // Start Hyper
  const hyper = new HyperProcess();
  const dbPath = join(TEST_DIR, 'benchmark.hyper');
  const conn = await Connection.connect(
    hyper.endpoint,
    dbPath,
    CreateMode.CreateAndReplace
  );

  // Create table (same schema as Rust benchmark)
  const tableDef = new TableDefinition('measurements');
  tableDef.addColumn('id', SqlType.int(), false);
  tableDef.addColumn('sensor_id', SqlType.int(), true);
  tableDef.addColumn('value', SqlType.double(), true);
  tableDef.addColumn('timestamp', SqlType.bigInt(), true);

  const catalog = new Catalog(conn);
  await catalog.createTable(tableDef);

  const results = [];

  // --- INSERT ---
  printHeader('INSERT BENCHMARKS');
  results.push(await benchInsert(conn, tableDef, ROW_COUNT));
  console.log();

  // Re-populate via Arrow IPC on a fresh table so timing isn't distorted
  // by appending to the row-API rows.
  await conn.executeCommand('TRUNCATE measurements');
  results.push(await benchInsertArrow(conn, tableDef, ROW_COUNT));

  // Validate
  const countRows = await conn.executeQuery('SELECT COUNT(*) FROM measurements');
  const actualCount = countRows[0].getInt32(0);
  console.log(`\n  Validation: ${actualCount} rows in table (expected ${ROW_COUNT})`);

  // --- QUERY BENCHMARKS ---
  printHeader('QUERY BENCHMARKS');

  results.push(await benchFullScanEager(conn, ROW_COUNT));
  console.log();
  results.push(await benchFullScanStream(conn, ROW_COUNT));
  console.log();
  results.push(await benchFullScanChunked(conn, ROW_COUNT));
  console.log();
  results.push(await benchFilteredQuery(conn, ROW_COUNT));
  console.log();
  results.push(await benchAggregation(conn, ROW_COUNT));

  // --- COLUMNAR BENCHMARKS ---
  printHeader('COLUMNAR BENCHMARKS (optimized)');

  results.push(await benchColumnarFullScan(conn, ROW_COUNT));
  console.log();
  results.push(await benchColumnarFiltered(conn, ROW_COUNT));

  // --- ARROW BENCHMARKS ---
  printHeader('ARROW BENCHMARKS (IPC)');

  results.push(await benchArrowFullScan(conn, ROW_COUNT));
  console.log();
  results.push(await benchArrowFiltered(conn, ROW_COUNT));

  // --- SUMMARY ---
  printHeader('SUMMARY');
  console.log(
    `  ${pad('Benchmark', 25, true)} ${pad('Rows', 10)}  ${pad('Time (s)', 10)}  ${pad('Rows/sec', 14)}  ${pad('MB/s', 10)}`
  );
  console.log('  ' + '-'.repeat(75));
  for (const r of results) {
    console.log(
      `  ${pad(r.name, 25, true)} ${pad(formatCount(r.rowCount), 10)}  ${pad(r.secs.toFixed(3), 10)}  ${pad(formatCount(Math.round(r.rowsPerSec)), 14)}  ${pad(r.throughput.toFixed(2), 10)}`
    );
  }

  // Clean up
  await conn.close();
  hyper.close();

  console.log('\nDone.');
}

main().catch((err) => {
  console.error('Benchmark failed:', err);
  process.exit(1);
});
