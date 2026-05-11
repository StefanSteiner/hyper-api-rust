#!/usr/bin/env npx tsx
/**
 * Complete API Tour — hyperdb-api-node (TypeScript)
 *
 * A comprehensive, fully-typed example demonstrating every feature of the
 * hyperdb-api-node bindings. Builds a realistic IoT sensor monitoring application.
 *
 * Run:
 *   HYPERD_PATH=/path/to/hyperd npx tsx examples/complete-api-tour.mts
 *
 * Prerequisites:
 *   npm install apache-arrow tsx
 */

import { strict as assert } from 'assert';
import { existsSync, mkdirSync, writeFileSync, readFileSync, unlinkSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';
import { createRequire } from 'module';
import { tableFromIPC, vectorFromArray, makeTable, Int32, Float64, Utf8 } from 'apache-arrow';
import type { Table, Schema, Field, RecordBatch } from 'apache-arrow';

const require = createRequire(import.meta.url);
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
} = require('../index.js') as typeof import('../index.js');

import type {
  RowData,
  ColumnarChunk,
  ResultColumnInfo,
} from '../index.js';

import {
  tableFromQuery,
  exportTable,
  batchesFromQuery,
  queryToArrowFile,
  insertFromTable,
  querySchema,
} from '../arrow.mjs';

import { ConnectionPool } from '../pool.mjs';

const __dirname: string = dirname(fileURLToPath(import.meta.url));
const OUT_DIR: string = join(__dirname, '..', 'test_results');
if (!existsSync(OUT_DIR)) mkdirSync(OUT_DIR, { recursive: true });

// ─── Helpers ─────────────────────────────────────────────────────────────────

let sectionNum: number = 0;
function section(title: string): void {
  sectionNum++;
  console.log(`\n${'━'.repeat(70)}`);
  console.log(`  ${sectionNum}. ${title}`);
  console.log(`${'━'.repeat(70)}`);
}

function log(msg: string): void { console.log(`  ${msg}`); }

// ─── Main ────────────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  console.log('╔══════════════════════════════════════════════════════════════════════╗');
  console.log('║          hyperdb-api-node — Complete API Tour (TypeScript)             ║');
  console.log('║          IoT Sensor Monitoring Application                         ║');
  console.log('╚══════════════════════════════════════════════════════════════════════╝');

  // ═══════════════════════════════════════════════════════════════════════════
  // 1. HyperProcess — start a local Hyper server
  // ═══════════════════════════════════════════════════════════════════════════

  section('HyperProcess — Start a local Hyper server');

  const hyper = new HyperProcess();
  log(`Endpoint:  ${hyper.endpoint}`);
  log(`Is open:   ${hyper.isOpen}`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 2. Connection — connect to a database
  // ═══════════════════════════════════════════════════════════════════════════

  section('Connection — Connect to a database');

  const dbPath: string = join(OUT_DIR, 'iot_demo_ts.hyper');
  let conn = await Connection.connect(hyper.endpoint, dbPath, CreateMode.CreateAndReplace);
  log(`Database:  ${conn.database}`);
  log(`Is alive:  ${conn.isAlive}`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 3. ConnectionBuilder — advanced connection options
  // ═══════════════════════════════════════════════════════════════════════════

  section('ConnectionBuilder — Advanced connection options');

  const conn2 = await new ConnectionBuilder(hyper.endpoint)
    .database(join(OUT_DIR, 'iot_demo_builder_ts.hyper'))
    .createMode(CreateMode.CreateAndReplace)
    .loginTimeout(10_000)
    .build();
  log(`Connected via builder: ${conn2.database}`);
  await conn2.close();

  // ═══════════════════════════════════════════════════════════════════════════
  // 4. SqlType & TableDefinition — define schemas in code
  // ═══════════════════════════════════════════════════════════════════════════

  section('SqlType & TableDefinition — Define schemas in code');

  const sensorsDef: InstanceType<typeof TableDefinition> = new TableDefinition('sensors');
  sensorsDef.withSchema('Extract');
  sensorsDef.addColumn('sensor_id', SqlType.int(), false);
  sensorsDef.addColumn('name', SqlType.text(), false);
  sensorsDef.addColumn('location', SqlType.text(), true);
  sensorsDef.addColumn('active', SqlType.bool(), false);
  log(`Table "${sensorsDef.name}" — ${sensorsDef.columnCount} columns`);
  log(`SQL: ${sensorsDef.toCreateSql()}`);

  const readingsDef = new TableDefinition('readings');
  readingsDef.withSchema('Extract');
  readingsDef.addColumn('reading_id', SqlType.bigInt(), false);
  readingsDef.addColumn('sensor_id', SqlType.int(), false);
  readingsDef.addColumn('temperature', SqlType.double(), true);
  readingsDef.addColumn('humidity', SqlType.double(), true);
  readingsDef.addColumn('pressure', SqlType.double(), true);
  readingsDef.addColumn('battery_pct', SqlType.smallInt(), true);
  readingsDef.addColumn('recorded_at', SqlType.bigInt(), false);
  log(`Table "${readingsDef.name}" — ${readingsDef.columnCount} columns`);

  const alertsDef = new TableDefinition('alerts');
  alertsDef.withSchema('Extract');
  alertsDef.addColumn('alert_id', SqlType.int(), false);
  alertsDef.addColumn('sensor_id', SqlType.int(), false);
  alertsDef.addColumn('severity', SqlType.text(), false);
  alertsDef.addColumn('message', SqlType.text(), false);
  alertsDef.addColumn('acknowledged', SqlType.bool(), false);

  // ═══════════════════════════════════════════════════════════════════════════
  // 5. Catalog — schema & table management
  // ═══════════════════════════════════════════════════════════════════════════

  section('Catalog — Schema & table management (DDL)');

  let catalog = new Catalog(conn);

  // Create the "Extract" schema (required by Tableau)
  await catalog.createSchema('Extract');
  await conn.executeCommand('SET search_path TO "Extract", public');
  log(`Created schema "Extract" (Tableau convention) + set search_path`);

  // Also demo a custom schema
  await catalog.createSchema('monitoring');
  log(`Created schema "monitoring"`);

  const schemas: string[] = await catalog.getSchemaNames();
  log(`All schemas: ${JSON.stringify(schemas)}`);

  await catalog.createTable(sensorsDef);
  await catalog.createTable(readingsDef);
  await catalog.createTable(alertsDef);
  log(`Created tables in "Extract" schema: sensors, readings, alerts`);

  const tables: string[] = await catalog.getTableNames('Extract');
  log(`Tables in "Extract": ${JSON.stringify(tables)}`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 6. RowInserter — bulk insert via COPY protocol
  // ═══════════════════════════════════════════════════════════════════════════

  section('RowInserter — Bulk insert via COPY protocol');

  const sensorInserter = new RowInserter(conn, sensorsDef);
  sensorInserter.addRow([1, 'Rooftop-A',      'Building 1, Roof',    true]);
  sensorInserter.addRow([2, 'Basement-B',     'Building 1, B1',      true]);
  sensorInserter.addRow([3, 'Garden-C',       'Building 2, Garden',  true]);
  sensorInserter.addRow([4, 'Server-Room',    'Building 1, Floor 3', true]);
  sensorInserter.addRow([5, 'Decommissioned', null,                  false]);
  log(`Buffered ${sensorInserter.bufferedRowCount} sensor rows`);
  const sensorCount: number = await sensorInserter.execute();
  log(`Inserted ${sensorCount} sensors`);

  const readingInserter = new RowInserter(conn, readingsDef);
  const BATCH_SIZE: number = 50_000;
  const TOTAL_READINGS: number = 100_000;
  const startTime: number = performance.now();

  for (let offset = 0; offset < TOTAL_READINGS; offset += BATCH_SIZE) {
    const count: number = Math.min(BATCH_SIZE, TOTAL_READINGS - offset);
    const batch: Array<Array<number | null>> = new Array(count);
    for (let i = 0; i < count; i++) {
      const id: number = offset + i;
      const sensorId: number = (id % 4) + 1;
      batch[i] = [
        id, sensorId,
        20.0 + Math.sin(id * 0.01) * 10,
        40.0 + Math.cos(id * 0.02) * 20,
        sensorId === 3 ? null : 1013.25 + id * 0.001,
        100 - Math.floor(id / 5000),
        1700000000000 + id * 60_000,
      ];
    }
    readingInserter.addRows(batch);
  }

  const readingsInserted: number = await readingInserter.execute();
  const insertMs: number = performance.now() - startTime;
  log(`Inserted ${readingsInserted.toLocaleString()} readings in ${(insertMs / 1000).toFixed(2)}s`);
  log(`Throughput: ${(readingsInserted / (insertMs / 1000)).toFixed(0)} rows/sec`);

  const alertInserter = new RowInserter(conn, alertsDef);
  alertInserter.addRows([
    [1, 1, 'warning',  'Temperature above 28°C',       false],
    [2, 2, 'critical', 'Humidity exceeds 80%',          false],
    [3, 3, 'info',     'Battery below 50%',             true],
    [4, 1, 'warning',  'Pressure drop detected',        false],
    [5, 4, 'critical', 'Sensor offline for >1 hour',    false],
  ]);
  await alertInserter.execute();
  log(`Inserted 5 alerts`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 7. executeCommand — run DML statements
  // ═══════════════════════════════════════════════════════════════════════════

  section('executeCommand — Run DML statements');

  const affected: number = await conn.executeCommand(
    "INSERT INTO alerts VALUES (6, 2, 'info', 'Routine maintenance scheduled', true)"
  );
  log(`Inserted ${affected} alert via executeCommand`);

  const updated: number = await conn.executeCommand(
    "UPDATE sensors SET location = 'Building 1, Floor 3, Rack A' WHERE sensor_id = 4"
  );
  log(`Updated ${updated} sensor location`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 8. executeQuery — eager row-oriented queries
  // ═══════════════════════════════════════════════════════════════════════════

  section('executeQuery — Eager row-oriented queries');

  const sensors: RowData[] = await conn.executeQuery('SELECT * FROM sensors ORDER BY sensor_id');
  log(`Retrieved ${sensors.length} sensors:\n`);
  log('  ID  Name               Location                      Active');
  log('  ' + '─'.repeat(65));
  for (const row of sensors) {
    const id: string = String(row.getInt32(0)).padStart(3);
    const name: string = (row.getString(1) ?? '').padEnd(18);
    const loc: string = (row.getString(2) ?? '(none)').padEnd(30);
    const active: string = row.getBool(3) ? '✓' : '✗';
    log(`  ${id}  ${name} ${loc} ${active}`);
  }

  // ═══════════════════════════════════════════════════════════════════════════
  // 9. RowData — typed accessors & null handling
  // ═══════════════════════════════════════════════════════════════════════════

  section('RowData — Typed accessors & null handling');

  const sampleRows: RowData[] = await conn.executeQuery(
    'SELECT * FROM readings WHERE reading_id < 5 ORDER BY reading_id'
  );
  log('RowData accessor methods:');
  for (const row of sampleRows) {
    log(`  reading_id = ${row.getInt64(0)} (getInt64)`);
    log(`  sensor_id  = ${row.getInt32(1)} (getInt32)`);
    log(`  temp       = ${row.getFloat64(2)?.toFixed(2)} (getFloat64)`);
    log(`  pressure   = ${row.isNull(4) ? 'NULL' : row.getFloat64(4)?.toFixed(2)} (isNull + getFloat64)`);
    log(`  as string  = ${row.getString(2)} (getString — converts any type)`);
    log(`  columns    = ${row.columnCount} (columnCount)`);
    log('');
    break;
  }

  const schemaInfo: ResultColumnInfo[] = await conn.querySchema('SELECT * FROM readings');
  log('Column schema:');
  for (const col of schemaInfo) {
    log(`  [${col.index}] ${col.name.padEnd(14)} ${col.typeName}`);
  }

  // ═══════════════════════════════════════════════════════════════════════════
  // 10. Tagged template literals + parameterized queries
  // ═══════════════════════════════════════════════════════════════════════════

  section('Tagged templates & parameterized queries');

  // Tagged template — the modern way
  const severity: string = 'critical';
  const ack: boolean = false;
  const critAlerts: RowData[] = await conn.sql`SELECT * FROM alerts WHERE severity = ${severity} AND acknowledged = ${ack}`;
  log('Tagged template (conn.sql`...`):');
  for (const row of critAlerts) {
    log(`  Alert #${row.getInt32(0)}: [${row.getString(2)}] ${row.getString(3)}`);
  }

  // Tagged template command
  await conn.command`INSERT INTO alerts VALUES (${7}, ${3}, ${'warning'}, ${'Battery critically low'}, ${false})`;
  log('\nTagged template command: added alert #7');

  // $1/$2 style
  const hotStmt = await conn.prepare(
    `SELECT reading_id, sensor_id, temperature FROM readings
     WHERE temperature > $1 AND temperature < $2 ORDER BY temperature DESC LIMIT 5`
  );
  const hotReadings: RowData[] = await hotStmt.query([29.0, 30.0]);
  await hotStmt.close();
  log(`\nPrepared statement ($1/$2): ${hotReadings.length} hot readings`);
  for (const row of hotReadings) {
    log(`  Reading #${row.getInt64(0)}: sensor=${row.getInt32(1)}, temp=${row.getFloat64(2)?.toFixed(3)}°C`);
  }

  // SQL injection safety
  const tricky: string = "O'Brien's \"special\" sensor";
  const trickyResult: RowData[] = await conn.sql`SELECT * FROM sensors WHERE name = ${tricky}`;
  log(`\nSQL injection test: "${tricky}" → ${trickyResult.length} rows (safe)`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 11. QueryStream — streaming large result sets
  // ═══════════════════════════════════════════════════════════════════════════

  section('QueryStream — Streaming large result sets');

  log('Chunk-level iteration:');
  const stream1 = conn.executeQueryStream('SELECT * FROM readings ORDER BY reading_id');
  let chunkCount: number = 0;
  let totalRows: number = 0;
  let chunk: RowData[] | null;
  while ((chunk = await stream1.nextChunk()) !== null) {
    chunkCount++;
    totalRows += chunk.length;
  }
  log(`  ${chunkCount} chunks, ${totalRows.toLocaleString()} total rows`);

  log('\nAsync iterator (for await...of):');
  const stream2 = conn.executeQueryStream(
    'SELECT sensor_id, temperature FROM readings ORDER BY reading_id LIMIT 10'
  );
  let iterCount: number = 0;
  for await (const row of stream2) {
    if (iterCount < 3) {
      log(`  sensor=${row.getInt32(0)}, temp=${row.getFloat64(1)?.toFixed(2)}°C`);
    }
    iterCount++;
  }
  log(`  ... ${iterCount} rows iterated`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 12. ColumnarStream — high-performance columnar access
  // ═══════════════════════════════════════════════════════════════════════════

  section('ColumnarStream — High-performance columnar access');

  const colStart: number = performance.now();
  const colStream = conn.executeQueryColumnar('SELECT * FROM readings ORDER BY reading_id');
  let colRows: number = 0;
  let tempSum: number = 0;
  let colChunk: ColumnarChunk | null;
  while ((colChunk = await colStream.nextChunk()) !== null) {
    colRows += colChunk.rowCount;
    const temps: number[] = colChunk.getFloat64Column(2);
    for (let i = 0; i < temps.length; i++) tempSum += temps[i];
  }
  const colMs: number = performance.now() - colStart;
  log(`Scanned ${colRows.toLocaleString()} rows in ${(colMs / 1000).toFixed(3)}s`);
  log(`Avg temperature: ${(tempSum / colRows).toFixed(2)}°C`);
  log(`Throughput: ${(colRows / (colMs / 1000) / 1e6).toFixed(1)}M rows/sec`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 13. Arrow IPC — raw buffer API
  // ═══════════════════════════════════════════════════════════════════════════

  section('Arrow IPC — Raw buffer API');

  const arrowBuf: Buffer = await conn.executeQueryToArrow(
    'SELECT sensor_id, temperature, humidity FROM readings LIMIT 1000'
  );
  log(`executeQueryToArrow: ${arrowBuf.byteLength.toLocaleString()} bytes`);

  const sensorBuf: Buffer = await conn.exportTableToArrow('sensors');
  log(`exportTableToArrow('sensors'): ${sensorBuf.byteLength} bytes`);

  const arrowFilePath: string = join(OUT_DIR, 'readings_sample_ts.arrows');
  writeFileSync(arrowFilePath, arrowBuf);
  const verifyTable: Table = tableFromIPC(readFileSync(arrowFilePath));
  log(`Verified: ${verifyTable.numRows} rows, ${verifyTable.numCols} columns`);
  try { unlinkSync(arrowFilePath); } catch {}

  // ═══════════════════════════════════════════════════════════════════════════
  // 14. Arrow convenience — full apache-arrow integration
  // ═══════════════════════════════════════════════════════════════════════════

  section('Arrow convenience — Full apache-arrow integration');

  const arrowTable: Table = await tableFromQuery(conn,
    `SELECT sensor_id, AVG(temperature) AS avg_temp, AVG(humidity) AS avg_hum,
            COUNT(*) AS reading_count
     FROM readings GROUP BY sensor_id ORDER BY sensor_id`
  );
  log('Sensor statistics via Arrow Table:');
  log(`  Schema: ${arrowTable.schema.fields.map((f: Field) => `${f.name}:${f.type}`).join(', ')}`);
  for (let i = 0; i < arrowTable.numRows; i++) {
    const sid = arrowTable.getChild('sensor_id')!.get(i);
    const temp = arrowTable.getChild('avg_temp')!.get(i);
    const hum = arrowTable.getChild('avg_hum')!.get(i);
    const cnt = arrowTable.getChild('reading_count')!.get(i);
    log(`  Sensor ${sid}: temp=${temp.toFixed(1)}°C, hum=${hum.toFixed(1)}%, readings=${cnt}`);
  }

  const allSensors: Table = await exportTable(conn, 'sensors');
  log(`\nexportTable('sensors'): ${allSensors.numRows} rows`);

  const batches: RecordBatch[] = await batchesFromQuery(conn, 'SELECT * FROM readings');
  log(`batchesFromQuery: ${batches.length} batch(es), ${batches[0]?.numRows} rows in first`);

  const readingsArrowSchema: Schema = await querySchema(conn, 'SELECT * FROM readings');
  log(`\nquerySchema (no data fetch):`);
  for (const f of readingsArrowSchema.fields) {
    log(`  ${f.name.padEnd(14)} → ${f.type}${f.nullable ? ' (nullable)' : ''}`);
  }

  // insertFromTable — round-trip
  log('\ninsertFromTable — Arrow Table → Hyper:');
  const maintenanceDef = new TableDefinition('maintenance_log');
  maintenanceDef.withSchema('Extract');
  // Declared nullable to match apache-arrow's default Arrow schema
  // nullability; Hyper's COPY validates nullability strictly.
  maintenanceDef.addColumn('log_id', SqlType.int(), true);
  maintenanceDef.addColumn('sensor_id', SqlType.int(), true);
  maintenanceDef.addColumn('action', SqlType.text(), true);
  maintenanceDef.addColumn('cost', SqlType.double(), true);
  await catalog.createTable(maintenanceDef);

  const maintenanceData: Table = makeTable({
    log_id:    vectorFromArray([1, 2, 3, 4], new Int32()),
    sensor_id: vectorFromArray([1, 2, 3, 4], new Int32()),
    action:    vectorFromArray(['Calibrated', 'Battery replaced', 'Cleaned', 'Firmware update'], new Utf8()),
    cost:      vectorFromArray([50.0, 25.0, null, 0.0], new Float64()),
  });
  const arrowInserted: number = await insertFromTable(conn, maintenanceDef, maintenanceData);
  log(`  Inserted ${arrowInserted} maintenance records from Arrow Table`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 15. ConnectionPool — pooled connections
  // ═══════════════════════════════════════════════════════════════════════════

  section('ConnectionPool — Pooled connections');

  const pool = new ConnectionPool(hyper.endpoint, dbPath, {
    min: 1,
    max: 5,
    idleTimeoutMs: 10_000,
    createMode: CreateMode.DoNotCreate,
  });
  log(`Pool created: min=1, max=5`);

  const countResult: RowData[] = await pool.query('SELECT COUNT(*) AS n FROM "Extract"."readings"');
  log(`pool.query: ${countResult[0].getInt64(0)!.toLocaleString()} readings`);

  const avgTemp: number = await pool.use(async (c) => {
    const rows: RowData[] = await c.executeQuery(
      'SELECT AVG(temperature) FROM "Extract"."readings" WHERE sensor_id = 1'
    );
    return rows[0].getFloat64(0)!;
  });
  log(`pool.use: avg temp for sensor 1 = ${avgTemp.toFixed(2)}°C`);

  log('\nConcurrent queries (3 parallel):');
  const t0: number = performance.now();
  const [r1, r2, r3] = await Promise.all([
    pool.query('SELECT COUNT(*) FROM "Extract"."readings" WHERE sensor_id = 1'),
    pool.query('SELECT COUNT(*) FROM "Extract"."readings" WHERE sensor_id = 2'),
    pool.query('SELECT COUNT(*) FROM "Extract"."readings" WHERE sensor_id = 3'),
  ]);
  log(`  Sensor 1: ${r1[0].getInt64(0)!.toLocaleString()} readings`);
  log(`  Sensor 2: ${r2[0].getInt64(0)!.toLocaleString()} readings`);
  log(`  Sensor 3: ${r3[0].getInt64(0)!.toLocaleString()} readings`);
  log(`  Completed in ${(performance.now() - t0).toFixed(0)}ms`);
  log(`  Pool stats: size=${pool.size}, idle=${pool.idle}, active=${pool.active}`);

  await pool.close();
  log('Pool closed ✓');

  // ═══════════════════════════════════════════════════════════════════════════
  // 16. Row projection + BigInt
  // ═══════════════════════════════════════════════════════════════════════════

  section('Row projection + BigInt');

  const jsonRows: RowData[] = await conn.executeQuery(
    'SELECT * FROM sensors ORDER BY sensor_id LIMIT 2'
  );
  const schemaCols: ResultColumnInfo[] = await conn.querySchema(
    'SELECT * FROM sensors'
  );
  const namedRow = Object.fromEntries(
    schemaCols.map((c: ResultColumnInfo, i: number) => [
      c.name,
      jsonRows[0].isNull(i) ? null : jsonRows[0].getString(i),
    ])
  );
  log(`row → JSON object: ${JSON.stringify(namedRow)}`);

  await conn.executeCommand('CREATE TABLE bigint_test (id BIGINT NOT NULL)');
  await conn.executeCommand('INSERT INTO bigint_test VALUES (9007199254740993)');
  const bigRows: RowData[] = await conn.executeQuery('SELECT id FROM bigint_test');
  const bigVal: bigint | null = bigRows[0].getBigInt(0);
  log(`getBigInt: ${bigVal} (type: ${typeof bigVal})`);
  await conn.executeCommand('DROP TABLE bigint_test');

  // ═══════════════════════════════════════════════════════════════════════════
  // 18. Error handling
  // ═══════════════════════════════════════════════════════════════════════════

  section('Error handling');

  try {
    await conn.executeQuery('SELECT * FROM nonexistent_table');
  } catch (err: unknown) {
    log(`Query error caught: "${(err as Error).message.split('\n')[0]}"`);
  }

  await conn.close();
  conn = await Connection.connect(hyper.endpoint, dbPath, CreateMode.DoNotCreate);
  await conn.executeCommand('SET search_path TO "Extract", public');
  catalog = new Catalog(conn);

  try {
    await pool.query('SELECT 1');
  } catch (err: unknown) {
    log(`Pool-after-close: "${(err as Error).message}"`);
  }

  // ═══════════════════════════════════════════════════════════════════════════
  // Cleanup
  // ═══════════════════════════════════════════════════════════════════════════

  section('Cleanup');

  await conn.close();
  log('Connection closed');

  hyper.close();
  log('Hyper server stopped');
  log(`Data persisted in: ${dbPath}`);

  console.log('\n╔══════════════════════════════════════════════════════════════════════╗');
  console.log('║  ✓ Complete API tour (TypeScript) finished successfully!            ║');
  console.log('╚══════════════════════════════════════════════════════════════════════╝\n');
}

main().catch((err: Error) => {
  console.error('\nExample failed:', err);
  process.exit(1);
});
