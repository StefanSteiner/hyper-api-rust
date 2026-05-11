#!/usr/bin/env node
/**
 * Arrow Analytics Example
 *
 * Demonstrates the full Apache Arrow integration with hyperdb-api-node:
 *   1. Create and populate a sales database
 *   2. Query data as Arrow Tables — inspect schema, access typed arrays
 *   3. Export to .arrow file (Arrow IPC file format)
 *   4. Round-trip: build an Arrow Table in JS, insert it into Hyper
 *   5. Cross-tool interop: load the .arrow file with any Arrow-compatible tool
 *
 * Prerequisites:
 *   npm install apache-arrow
 *
 * Run:
 *   HYPERD_PATH=/path/to/hyperd node examples/arrow-analytics.mjs
 */

import { existsSync, mkdirSync, writeFileSync, readFileSync, unlinkSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

import {
  tableFromArrays,
  tableFromIPC,
  tableToIPC,
  vectorFromArray,
  makeTable,
  Float64,
  Int32,
  Utf8,
} from 'apache-arrow';

import { createRequire } from 'module';
const require = createRequire(import.meta.url);
const {
  HyperProcess,
  Connection,
  Catalog,
  ArrowInserter,
  TableDefinition,
  SqlType,
  CreateMode,
} = require('../index.js');

import {
  tableFromQuery,
  exportTable,
  batchesFromQuery,
  queryToArrowFile,
  insertFromTable,
  querySchema,
} from '../arrow.mjs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const OUT_DIR = join(__dirname, '..', 'test_results');

function header(title) {
  console.log(`\n${'─'.repeat(60)}`);
  console.log(`  ${title}`);
  console.log(`${'─'.repeat(60)}`);
}

async function main() {
  if (!existsSync(OUT_DIR)) mkdirSync(OUT_DIR, { recursive: true });

  console.log('Arrow Analytics Example');
  console.log('=======================\n');

  // ── 1. Set up Hyper and create a sales database ──────────────────────

  header('1. Create sales database');

  const hyper = new HyperProcess();
  const dbPath = join(OUT_DIR, 'arrow_example.hyper');
  const conn = await Connection.connect(hyper.endpoint, dbPath, CreateMode.CreateAndReplace);

  // Columns declared nullable so the Arrow schema nullability matches —
  // apache-arrow's vectorFromArray produces nullable-by-default fields.
  const salesDef = new TableDefinition('sales');
  salesDef.addColumn('order_id', SqlType.int(), true);
  salesDef.addColumn('product', SqlType.text(), true);
  salesDef.addColumn('region', SqlType.text(), true);
  salesDef.addColumn('quantity', SqlType.int(), true);
  salesDef.addColumn('unit_price', SqlType.double(), true);
  salesDef.addColumn('discount', SqlType.double(), true);

  const catalog = new Catalog(conn);
  await catalog.createTable(salesDef);

  // Build an Arrow table in memory, serialize to IPC, stream into Hyper
  // via ArrowInserter — no per-row JS↔Rust conversion.
  const N = 1_000_000;
  const products = ['Widget', 'Gadget', 'Doohickey', 'Thingamajig'];
  const regions = ['North', 'South', 'East', 'West'];

  const orderIds = new Int32Array(N);
  const quantities = new Int32Array(N);
  const unitPrices = new Float64Array(N);
  const productArr = new Array(N);
  const regionArr = new Array(N);
  const discounts = new Float64Array(N);
  for (let i = 0; i < N; i++) {
    orderIds[i] = i;
    productArr[i] = products[i % products.length];
    regionArr[i] = regions[i % regions.length];
    quantities[i] = 1 + (i % 20);
    unitPrices[i] = 5.0 + (i % 50) * 0.5;
    discounts[i] = i % 7 === 0 ? NaN : (i % 30) * 0.01;
  }

  const salesArrow = makeTable({
    order_id:   vectorFromArray(orderIds, new Int32()),
    product:    vectorFromArray(productArr, new Utf8()),
    region:     vectorFromArray(regionArr, new Utf8()),
    quantity:   vectorFromArray(quantities, new Int32()),
    unit_price: vectorFromArray(unitPrices, new Float64()),
    discount:   vectorFromArray(discounts, new Float64()),
  });

  const ingestStart = performance.now();
  const arrowInserter = ArrowInserter.create(conn, salesDef);
  await arrowInserter.insertRaw(Buffer.from(tableToIPC(salesArrow, 'stream')));
  const inserted = await arrowInserter.execute();
  const ingestMs = performance.now() - ingestStart;

  console.log(`Created "sales" table with ${inserted.toLocaleString()} rows via ArrowInserter`);
  console.log(`Ingest time: ${(ingestMs / 1000).toFixed(2)}s (${(inserted / (ingestMs / 1000)).toFixed(0)} rows/sec)`);
  console.log('Columns: order_id (INT), product (TEXT), region (TEXT),');
  console.log('         quantity (INT), unit_price (DOUBLE), discount (DOUBLE nullable)');

  // ── 2. Query as Arrow Table — schema, typed arrays, filtering ────────

  header('2. Query as Arrow Table');

  const table = await tableFromQuery(conn, 'SELECT * FROM sales ORDER BY order_id');

  console.log(`Rows: ${table.numRows}, Columns: ${table.numCols}`);
  console.log(`Schema:`);
  for (const field of table.schema.fields) {
    console.log(`  ${field.name.padEnd(12)} ${field.type} ${field.nullable ? '(nullable)' : ''}`);
  }

  // Access columns as typed arrays — zero-copy from Arrow buffers
  const qtyCol = table.getChild('quantity').toArray();
  const prices = table.getChild('unit_price').toArray();
  const totalRevenue = qtyCol.reduce((sum, q, i) => sum + q * prices[i], 0);

  console.log(`\nTotal revenue (quantity × unit_price): $${totalRevenue.toFixed(2)}`);
  console.log(`Avg quantity: ${(qtyCol.reduce((a, b) => a + b, 0) / qtyCol.length).toFixed(1)}`);

  // ── 3. Query with aggregation — Arrow makes GROUP BY results easy ────

  header('3. Aggregation via Arrow');

  const regionStats = await tableFromQuery(conn, `
    SELECT
      region,
      COUNT(*) AS order_count,
      SUM(quantity) AS total_qty,
      ROUND(AVG(unit_price), 2) AS avg_price,
      ROUND(SUM(quantity * unit_price), 2) AS revenue
    FROM sales
    GROUP BY region
    ORDER BY revenue DESC
  `);

  console.log('Revenue by region:');
  for (let i = 0; i < regionStats.numRows; i++) {
    const region = regionStats.getChild('region').get(i);
    const count = regionStats.getChild('order_count').get(i);
    const qty = regionStats.getChild('total_qty').get(i);
    const price = regionStats.getChild('avg_price').get(i);
    const rev = regionStats.getChild('revenue').get(i);
    console.log(`  ${region.padEnd(6)} ${String(count).padStart(6)} orders  ${String(qty).padStart(7)} qty  $${String(price).padStart(6)} avg  $${String(rev).padStart(12)} revenue`);
  }

  // ── 4. RecordBatch-level access ──────────────────────────────────────

  header('4. RecordBatch streaming');

  const batches = await batchesFromQuery(conn, 'SELECT * FROM sales');
  console.log(`Got ${batches.length} RecordBatch(es)`);
  for (const [i, batch] of batches.entries()) {
    console.log(`  Batch ${i}: ${batch.numRows} rows, ${batch.numCols} columns`);
  }

  // ── 5. Export to .arrow file ─────────────────────────────────────────

  header('5. Export to .arrow file');

  const arrowFilePath = join(OUT_DIR, 'sales.arrow');
  const arrowBytes = await queryToArrowFile(conn, 'SELECT * FROM sales');
  writeFileSync(arrowFilePath, arrowBytes);
  console.log(`Wrote ${arrowFilePath}`);
  console.log(`File size: ${(arrowBytes.byteLength / 1024).toFixed(1)} KB`);

  // Verify: read it back
  const reloaded = tableFromIPC(readFileSync(arrowFilePath));
  console.log(`Re-read: ${reloaded.numRows} rows, ${reloaded.numCols} columns ✓`);

  // ── 6. Build Arrow Table in JS and insert into Hyper ─────────────────

  header('6. Insert from Arrow Table');

  // Create a new table for the round-trip
  const metricsDef = new TableDefinition('metrics');
  metricsDef.addColumn('sensor_id', SqlType.int(), true);
  metricsDef.addColumn('label', SqlType.text(), true);
  metricsDef.addColumn('value', SqlType.double(), true);
  await catalog.createTable(metricsDef);

  // Build an Arrow Table in pure JS
  const arrowMetrics = makeTable({
    sensor_id: vectorFromArray([1, 2, 3, 4, 5], new Int32()),
    label: vectorFromArray(['temp', 'humidity', 'pressure', 'wind', 'rain'], new Utf8()),
    value: vectorFromArray([22.5, 65.0, 1013.25, 12.3, 0.0], new Float64()),
  });

  console.log(`Arrow Table to insert: ${arrowMetrics.numRows} rows`);
  const insertCount = await insertFromTable(conn, metricsDef, arrowMetrics);
  console.log(`Inserted: ${insertCount} rows`);

  // Verify round-trip
  const roundTrip = await exportTable(conn, 'metrics');
  console.log(`Round-trip verification:`);
  for (let i = 0; i < roundTrip.numRows; i++) {
    const sid = roundTrip.getChild('sensor_id').get(i);
    const label = roundTrip.getChild('label').get(i);
    const val = roundTrip.getChild('value').get(i);
    console.log(`  sensor=${sid}  label=${label.padEnd(10)}  value=${val}`);
  }

  // ── 7. Schema introspection ──────────────────────────────────────────

  header('7. Schema introspection (no data fetch)');

  const schema = await querySchema(conn, 'SELECT * FROM sales');
  console.log('Sales table Arrow schema:');
  for (const field of schema.fields) {
    console.log(`  ${field.name.padEnd(12)} → Arrow type: ${field.type}`);
  }

  // ── 8. Raw IPC buffer usage (no apache-arrow needed) ─────────────────

  header('8. Raw IPC buffer (zero-dependency path)');

  const rawBuf = await conn.executeQueryToArrow(
    'SELECT order_id, product, quantity FROM sales LIMIT 5'
  );
  console.log(`Raw Arrow IPC stream: ${rawBuf.byteLength} bytes`);
  console.log('This Buffer can be passed to any Arrow-compatible tool:');
  console.log('  - apache-arrow: tableFromIPC(buf)');
  console.log('  - DuckDB: register_buffer() / read_arrow()');
  console.log('  - Write to .arrows file for Polars, pandas, etc.');

  // ── Cleanup ──────────────────────────────────────────────────────────

  await conn.close();
  hyper.close();

  // Clean up temp files
  // try { unlinkSync(arrowFilePath); } catch {}

  console.log('\n✓ All Arrow examples completed successfully.\n');
}

main().catch((err) => {
  console.error('Example failed:', err);
  process.exit(1);
});
