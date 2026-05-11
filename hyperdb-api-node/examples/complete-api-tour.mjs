#!/usr/bin/env node
/**
 * Complete API Tour — hyperdb-api-node
 *
 * A comprehensive example demonstrating every feature of the hyperdb-api-node
 * bindings. Builds a realistic IoT sensor monitoring application that:
 *
 *   1.  HyperProcess — start/stop a local Hyper server
 *   2.  Connection — connect to a database
 *   3.  ConnectionBuilder — advanced connection options
 *   4.  Catalog — schema & table management (DDL)
 *   5.  TableDefinition & SqlType — define table schemas in code
 *   6.  RowInserter / ArrowInserter — bulk insert via COPY protocol
 *   7.  executeCommand — run DML statements
 *   8.  executeQuery — eager row-oriented queries
 *   9.  RowData — typed column accessors, null handling
 *  10.  Parameterized queries — $1/$2 placeholders, SQL injection safety
 *  11.  QueryStream — streaming large result sets (chunk + async iterator)
 *  12.  ColumnarStream — high-performance columnar access
 *  13.  Arrow IPC — executeQueryToArrow, exportTableToArrow
 *  14.  Arrow convenience — tableFromQuery, insertFromTable, queryToArrowFile
 *  15.  ConnectionPool — pooled connections with auto acquire/release
 *  16.  Error handling — try/catch patterns
 *
 * Run:
 *   HYPERD_PATH=/path/to/hyperd node examples/complete-api-tour.mjs
 *
 * Prerequisites:
 *   npm install apache-arrow   (for Arrow sections)
 */

import { strict as assert } from 'assert';
import { existsSync, mkdirSync, writeFileSync, readFileSync, unlinkSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';
import { createRequire } from 'module';
import { tableFromIPC, vectorFromArray, makeTable, Int32, Float64, Utf8 } from 'apache-arrow';

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
} = require('../index.js');

import {
  tableFromQuery,
  exportTable,
  batchesFromQuery,
  queryToArrowFile,
  insertFromTable,
  querySchema,
} from '../arrow.mjs';

import { ConnectionPool } from '../pool.mjs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const OUT_DIR = join(__dirname, '..', 'test_results');
if (!existsSync(OUT_DIR)) mkdirSync(OUT_DIR, { recursive: true });

// ─── Helpers ─────────────────────────────────────────────────────────────────

let sectionNum = 0;
function section(title) {
  sectionNum++;
  console.log(`\n${'━'.repeat(70)}`);
  console.log(`  ${sectionNum}. ${title}`);
  console.log(`${'━'.repeat(70)}`);
}

function log(msg) { console.log(`  ${msg}`); }

// ─── Main ────────────────────────────────────────────────────────────────────

async function main() {
  console.log('╔══════════════════════════════════════════════════════════════════════╗');
  console.log('║          hyperdb-api-node — Complete API Tour                          ║');
  console.log('║          IoT Sensor Monitoring Application                         ║');
  console.log('╚══════════════════════════════════════════════════════════════════════╝');

  // ═══════════════════════════════════════════════════════════════════════════
  // 1. HyperProcess — start a local Hyper server
  // ═══════════════════════════════════════════════════════════════════════════

  section('HyperProcess — Start a local Hyper server');

  const hyper = new HyperProcess();
  log(`Endpoint:  ${hyper.endpoint}`);
  log(`Is open:   ${hyper.isOpen}`);
  log('The server runs as a child process and is stopped when close() is called.');

  // ═══════════════════════════════════════════════════════════════════════════
  // 2. Connection — connect to a database
  // ═══════════════════════════════════════════════════════════════════════════

  section('Connection — Connect to a database');

  const dbPath = join(OUT_DIR, 'iot_demo.hyper');
  let conn = await Connection.connect(
    hyper.endpoint,
    dbPath,
    CreateMode.CreateAndReplace,
  );
  log(`Database:  ${conn.database}`);
  log(`Is alive:  ${conn.isAlive}`);
  log('CreateAndReplace drops any existing database and creates a fresh one.');

  // ═══════════════════════════════════════════════════════════════════════════
  // 3. ConnectionBuilder — advanced connection options
  // ═══════════════════════════════════════════════════════════════════════════

  section('ConnectionBuilder — Advanced connection options');

  const conn2 = await new ConnectionBuilder(hyper.endpoint)
    .database(join(OUT_DIR, 'iot_demo_builder.hyper'))
    .createMode(CreateMode.CreateIfNotExists)
    .loginTimeout(10_000)
    .build();
  log(`Connected via builder: ${conn2.database}`);
  log('ConnectionBuilder supports: .user(), .password(), .loginTimeout(), .createMode(), .database()');
  await conn2.close();

  // ═══════════════════════════════════════════════════════════════════════════
  // 4. SqlType & TableDefinition — define schemas in code
  // ═══════════════════════════════════════════════════════════════════════════

  section('SqlType & TableDefinition — Define schemas in code');

  log('Available SqlTypes:');
  log('  SqlType.bool()          SqlType.smallInt()       SqlType.int()');
  log('  SqlType.bigInt()        SqlType.float()          SqlType.double()');
  log('  SqlType.numeric(18,2)   SqlType.text()           SqlType.varchar(255)');
  log('  SqlType.char(10)        SqlType.bytes()          SqlType.date()');
  log('  SqlType.time()          SqlType.timestamp()      SqlType.timestampTz()');
  log('  SqlType.interval()      SqlType.json()           SqlType.geography()');

  // Sensors table
  const sensorsDef = new TableDefinition('sensors');
  sensorsDef.addColumn('sensor_id', SqlType.int(), false);     // NOT NULL
  sensorsDef.addColumn('name', SqlType.text(), false);
  sensorsDef.addColumn('location', SqlType.text(), true);      // NULLABLE
  sensorsDef.addColumn('active', SqlType.bool(), false);

  log(`\nTable "${sensorsDef.name}" — ${sensorsDef.columnCount} columns`);
  log(`SQL: ${sensorsDef.toCreateSql()}`);

  // Readings table
  const readingsDef = new TableDefinition('readings');
  readingsDef.addColumn('reading_id', SqlType.bigInt(), false);
  readingsDef.addColumn('sensor_id', SqlType.int(), false);
  readingsDef.addColumn('temperature', SqlType.double(), true);
  readingsDef.addColumn('humidity', SqlType.double(), true);
  readingsDef.addColumn('pressure', SqlType.double(), true);
  readingsDef.addColumn('battery_pct', SqlType.smallInt(), true);
  readingsDef.addColumn('recorded_at', SqlType.bigInt(), false);

  log(`Table "${readingsDef.name}" — ${readingsDef.columnCount} columns`);

  // Alerts table (for parameterized query demos)
  const alertsDef = new TableDefinition('alerts');
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

  // Schema operations
  await catalog.createSchema('monitoring');
  log(`Created schema "monitoring"`);
  log(`Has schema "monitoring": ${await catalog.hasSchema('monitoring')}`);
  log(`Has schema "nonexistent": ${await catalog.hasSchema('nonexistent')}`);
  const schemas = await catalog.getSchemaNames();
  log(`All schemas: ${JSON.stringify(schemas)}`);

  // Create tables
  await catalog.createTable(sensorsDef);
  await catalog.createTable(readingsDef);
  await catalog.createTable(alertsDef);
  log(`\nCreated tables: sensors, readings, alerts`);

  // Table introspection
  log(`Has table "sensors": ${await catalog.hasTable('sensors')}`);
  const tables = await catalog.getTableNames('public');
  log(`Tables in "public": ${JSON.stringify(tables)}`);

  // createTableIfNotExists (no error on second call)
  await catalog.createTableIfNotExists(sensorsDef);
  log('createTableIfNotExists: safe to call repeatedly');

  // ═══════════════════════════════════════════════════════════════════════════
  // 6. RowInserter — bulk insert via COPY protocol
  // ═══════════════════════════════════════════════════════════════════════════

  section('RowInserter — Bulk insert via COPY protocol');

  // Insert sensors one at a time with addRow
  const sensorInserter = new RowInserter(conn, sensorsDef);
  sensorInserter.addRow([1, 'Rooftop-A',      'Building 1, Roof',    true]);
  sensorInserter.addRow([2, 'Basement-B',     'Building 1, B1',      true]);
  sensorInserter.addRow([3, 'Garden-C',       'Building 2, Garden',  true]);
  sensorInserter.addRow([4, 'Server-Room',    'Building 1, Floor 3', true]);
  sensorInserter.addRow([5, 'Decommissioned', null,                  false]);
  log(`Buffered ${sensorInserter.bufferedRowCount} sensor rows`);
  const sensorCount = await sensorInserter.execute();
  log(`Inserted ${sensorCount} sensors`);

  // Bulk insert readings with addRows (batched)
  const readingInserter = new RowInserter(conn, readingsDef);
  const batchSize = 50_000;
  const totalReadings = 100_000;
  const startTime = performance.now();

  for (let offset = 0; offset < totalReadings; offset += batchSize) {
    const count = Math.min(batchSize, totalReadings - offset);
    const batch = new Array(count);
    for (let i = 0; i < count; i++) {
      const id = offset + i;
      const sensorId = (id % 4) + 1; // sensors 1-4 (skip decommissioned #5)
      batch[i] = [
        id,                                              // reading_id
        sensorId,                                        // sensor_id
        20.0 + Math.sin(id * 0.01) * 10,                 // temperature
        40.0 + Math.cos(id * 0.02) * 20,                 // humidity
        sensorId === 3 ? null : 1013.25 + id * 0.001,    // pressure (null for outdoor)
        100 - Math.floor(id / 5000),                     // battery_pct
        1700000000000 + id * 60_000,                     // recorded_at (1-min intervals)
      ];
    }
    readingInserter.addRows(batch);
  }

  const readingsInserted = await readingInserter.execute();
  const insertMs = performance.now() - startTime;
  log(`Inserted ${readingsInserted.toLocaleString()} readings in ${(insertMs / 1000).toFixed(2)}s`);
  log(`Throughput: ${(readingsInserted / (insertMs / 1000)).toFixed(0)} rows/sec`);

  // Insert alerts
  const alertInserter = new RowInserter(conn, alertsDef);
  alertInserter.addRows([
    [1, 1, 'warning',  'Temperature above 28°C',       false],
    [2, 2, 'critical', 'Humidity exceeds 80%',          false],
    [3, 3, 'info',     'Battery below 50%',             true],
    [4, 1, 'warning',  "Pressure drop detected",        false],
    [5, 4, 'critical', 'Sensor offline for >1 hour',    false],
  ]);
  await alertInserter.execute();
  log(`Inserted 5 alerts`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 7. executeCommand — run DML statements
  // ═══════════════════════════════════════════════════════════════════════════

  section('executeCommand — Run DML statements');

  // Direct SQL INSERT
  const affected = await conn.executeCommand(
    "INSERT INTO alerts VALUES (6, 2, 'info', 'Routine maintenance scheduled', true)"
  );
  log(`Inserted ${affected} alert via executeCommand`);

  // UPDATE
  const updated = await conn.executeCommand(
    "UPDATE sensors SET location = 'Building 1, Floor 3, Rack A' WHERE sensor_id = 4"
  );
  log(`Updated ${updated} sensor location`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 8. executeQuery — eager row-oriented queries
  // ═══════════════════════════════════════════════════════════════════════════

  section('executeQuery — Eager row-oriented queries');

  const sensors = await conn.executeQuery('SELECT * FROM sensors ORDER BY sensor_id');
  log(`Retrieved ${sensors.length} sensors:\n`);
  log('  ID  Name               Location                      Active');
  log('  ' + '─'.repeat(65));
  for (const row of sensors) {
    const id = String(row.getInt32(0)).padStart(3);
    const name = (row.getString(1) ?? '').padEnd(18);
    const loc = (row.getString(2) ?? '(none)').padEnd(30);
    const active = row.getBool(3) ? '✓' : '✗';
    log(`  ${id}  ${name} ${loc} ${active}`);
  }

  // ═══════════════════════════════════════════════════════════════════════════
  // 9. RowData — typed accessors & null handling
  // ═══════════════════════════════════════════════════════════════════════════

  section('RowData — Typed accessors & null handling');

  const sampleRows = await conn.executeQuery(
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
    break; // Just show one row
  }

  // Schema metadata
  const schema = await conn.querySchema('SELECT * FROM readings');
  log('Column schema:');
  for (const col of schema) {
    log(`  [${col.index}] ${col.name.padEnd(14)} ${col.typeName}`);
  }

  // ═══════════════════════════════════════════════════════════════════════════
  // 10. Parameterized queries — SQL injection safety
  // ═══════════════════════════════════════════════════════════════════════════

  section('Prepared statements — server-side $1/$2 placeholders');

  // SELECT with multiple typed parameters
  log('SELECT with string + number params:');
  const critAlertsStmt = await conn.prepare(
    'SELECT * FROM alerts WHERE severity = $1 AND acknowledged = $2'
  );
  const critAlerts = await critAlertsStmt.query(['critical', false]);
  for (const row of critAlerts) {
    log(`  Alert #${row.getInt32(0)}: [${row.getString(2)}] ${row.getString(3)}`);
  }
  await critAlertsStmt.close();

  // Parameterized INSERT — prepared once, executed once.
  const insertAlertStmt = await conn.prepare(
    'INSERT INTO alerts VALUES ($1, $2, $3, $4, $5)'
  );
  await insertAlertStmt.execute([7, 3, 'warning', 'Battery critically low at 5%', false]);
  await insertAlertStmt.close();
  log('\nParameterized INSERT: added alert #7');

  // Parameterized UPDATE
  const ackStmt = await conn.prepare(
    'UPDATE alerts SET acknowledged = $1 WHERE alert_id = $2'
  );
  await ackStmt.execute([true, 2]);
  await ackStmt.close();
  log('Parameterized UPDATE: acknowledged alert #2');

  // Range query with parameterized floats — reuse the same prepared
  // statement for repeated calls with different values (the server
  // keeps its plan cache warm).
  const hotRangeStmt = await conn.prepare(
    `SELECT reading_id, sensor_id, temperature
     FROM readings
     WHERE temperature > $1 AND temperature < $2
     ORDER BY temperature DESC
     LIMIT 5`
  );
  const hotReadings = await hotRangeStmt.query([29.0, 30.0]);
  log(`\nReadings between 29°C and 30°C (top 5):`);
  for (const row of hotReadings) {
    log(`  Reading #${row.getInt64(0)}: sensor=${row.getInt32(1)}, temp=${row.getFloat64(2)?.toFixed(3)}°C`);
  }
  await hotRangeStmt.close();

  // SQL injection prevention — special characters never hit the SQL
  // parser; the parameter binds as a TEXT value.
  const tricky = "O'Brien's \"special\" sensor";
  const injStmt = await conn.prepare('SELECT * FROM sensors WHERE name = $1');
  const trickyResult = await injStmt.query([tricky]);
  await injStmt.close();
  log(`\nSQL injection test: input = "${tricky}"`);
  log(`Result: ${trickyResult.length} rows (bound as TEXT — no parsing, no crash)`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 11. QueryStream — streaming large result sets
  // ═══════════════════════════════════════════════════════════════════════════

  section('QueryStream — Streaming large result sets');

  // Chunk-level iteration (high performance)
  log('Chunk-level iteration:');
  const stream1 = conn.executeQueryStream('SELECT * FROM readings ORDER BY reading_id');
  let chunkCount = 0;
  let totalRows = 0;
  let chunk;
  while ((chunk = await stream1.nextChunk()) !== null) {
    chunkCount++;
    totalRows += chunk.length;
  }
  log(`  ${chunkCount} chunks, ${totalRows.toLocaleString()} total rows`);

  // Schema from stream
  const stream1b = conn.executeQueryStream('SELECT reading_id, temperature FROM readings LIMIT 1');
  await stream1b.nextChunk();
  const streamSchema = stream1b.getSchema();
  log(`  Stream schema: ${streamSchema?.map(c => `${c.name}(${c.typeName})`).join(', ')}`);

  // Async iterator (convenient row-level access)
  log('\nAsync iterator (for await...of):');
  const stream2 = conn.executeQueryStream(
    'SELECT sensor_id, temperature FROM readings ORDER BY reading_id LIMIT 10'
  );
  let iterCount = 0;
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

  const colStart = performance.now();
  const colStream = conn.executeQueryColumnar('SELECT * FROM readings ORDER BY reading_id');
  let colRows = 0;
  let tempSum = 0;
  let colChunk;
  while ((colChunk = await colStream.nextChunk()) !== null) {
    colRows += colChunk.rowCount;
    const temps = colChunk.getFloat64Column(2); // temperature column
    for (let i = 0; i < temps.length; i++) tempSum += temps[i];
  }
  const colMs = performance.now() - colStart;
  log(`Scanned ${colRows.toLocaleString()} rows in ${(colMs / 1000).toFixed(3)}s`);
  log(`Avg temperature: ${(tempSum / colRows).toFixed(2)}°C`);
  log(`Throughput: ${(colRows / (colMs / 1000) / 1e6).toFixed(1)}M rows/sec`);

  // Column accessors
  log('\nColumnarChunk accessors:');
  const colStream2 = conn.executeQueryColumnar('SELECT * FROM readings LIMIT 5');
  const c2 = await colStream2.nextChunk();
  log(`  rowCount:         ${c2.rowCount}`);
  log(`  columnCount:      ${c2.columnCount}`);
  log(`  getInt64Column:   [${c2.getInt64Column(0).slice(0, 3).join(', ')}...]`);
  log(`  getInt32Column:   [${c2.getInt32Column(1).slice(0, 3).join(', ')}...]`);
  log(`  getFloat64Column: [${c2.getFloat64Column(2).slice(0, 3).map(v => v.toFixed(2)).join(', ')}...]`);
  const nulls = c2.getNulls(4); // pressure (some nulls for outdoor sensor)
  log(`  getNulls(4):      [${nulls.slice(0, 5).join(', ')}] (pressure, null for outdoor)`);

  // ═══════════════════════════════════════════════════════════════════════════
  // 13. Arrow IPC — raw buffer API
  // ═══════════════════════════════════════════════════════════════════════════

  section('Arrow IPC — Raw buffer API (zero-dependency)');

  // executeQueryToArrow — returns raw Arrow IPC stream bytes
  const arrowBuf = await conn.executeQueryToArrow(
    'SELECT sensor_id, temperature, humidity FROM readings LIMIT 1000'
  );
  log(`executeQueryToArrow: ${arrowBuf.byteLength.toLocaleString()} bytes`);

  // exportTableToArrow — export an entire table
  const sensorBuf = await conn.exportTableToArrow('sensors');
  log(`exportTableToArrow('sensors'): ${sensorBuf.byteLength} bytes`);

  // Write to .arrows file (readable by DuckDB, Polars, pandas)
  const arrowFilePath = join(OUT_DIR, 'readings_sample.arrows');
  writeFileSync(arrowFilePath, arrowBuf);
  log(`Wrote ${arrowFilePath}`);

  // Verify: parse with tableFromIPC
  const verifyTable = tableFromIPC(readFileSync(arrowFilePath));
  log(`Verified: ${verifyTable.numRows} rows, ${verifyTable.numCols} columns`);
  try { unlinkSync(arrowFilePath); } catch {}

  // ═══════════════════════════════════════════════════════════════════════════
  // 14. Arrow convenience — full apache-arrow integration
  // ═══════════════════════════════════════════════════════════════════════════

  section('Arrow convenience — Full apache-arrow integration');

  // tableFromQuery — query → Arrow Table
  const arrowTable = await tableFromQuery(conn,
    `SELECT sensor_id, AVG(temperature) AS avg_temp, AVG(humidity) AS avg_hum,
            COUNT(*) AS reading_count
     FROM readings
     GROUP BY sensor_id
     ORDER BY sensor_id`
  );
  log('Sensor statistics via Arrow Table:');
  log(`  Schema: ${arrowTable.schema.fields.map(f => `${f.name}:${f.type}`).join(', ')}`);
  for (let i = 0; i < arrowTable.numRows; i++) {
    const sid = arrowTable.getChild('sensor_id').get(i);
    const temp = arrowTable.getChild('avg_temp').get(i);
    const hum = arrowTable.getChild('avg_hum').get(i);
    const cnt = arrowTable.getChild('reading_count').get(i);
    log(`  Sensor ${sid}: temp=${temp.toFixed(1)}°C, hum=${hum.toFixed(1)}%, readings=${cnt}`);
  }

  // exportTable — entire table as Arrow
  const allSensors = await exportTable(conn, 'sensors');
  log(`\nexportTable('sensors'): ${allSensors.numRows} rows`);

  // batchesFromQuery — RecordBatch array
  const batches = await batchesFromQuery(conn, 'SELECT * FROM readings');
  log(`batchesFromQuery: ${batches.length} batch(es), ${batches[0]?.numRows} rows in first`);

  // queryToArrowFile — Arrow IPC file format with footer
  const arrowFileBytes = await queryToArrowFile(conn, 'SELECT * FROM alerts');
  const alertsFilePath = join(OUT_DIR, 'alerts.arrow');
  writeFileSync(alertsFilePath, arrowFileBytes);
  log(`queryToArrowFile: wrote ${alertsFilePath} (${arrowFileBytes.byteLength} bytes)`);
  try { unlinkSync(alertsFilePath); } catch {}

  // querySchema — get Arrow schema without fetching data
  const readingsSchema = await querySchema(conn, 'SELECT * FROM readings');
  log(`\nquerySchema (no data fetch):`);
  for (const f of readingsSchema.fields) {
    log(`  ${f.name.padEnd(14)} → ${f.type}${f.nullable ? ' (nullable)' : ''}`);
  }

  // insertFromTable — round-trip: build Arrow Table → insert into Hyper
  log('\ninsertFromTable — Arrow Table → Hyper:');
  const maintenanceDef = new TableDefinition('maintenance_log');
  // Declared nullable to match the default nullability of apache-arrow's
  // vectorFromArray output — mismatch between Arrow schema nullability
  // and Hyper's NOT NULL constraint would fail the COPY.
  maintenanceDef.addColumn('log_id', SqlType.int(), true);
  maintenanceDef.addColumn('sensor_id', SqlType.int(), true);
  maintenanceDef.addColumn('action', SqlType.text(), true);
  maintenanceDef.addColumn('cost', SqlType.double(), true);
  await catalog.createTable(maintenanceDef);

  const maintenanceData = makeTable({
    log_id:    vectorFromArray([1, 2, 3, 4], new Int32()),
    sensor_id: vectorFromArray([1, 2, 3, 4], new Int32()),
    action:    vectorFromArray(['Calibrated', 'Battery replaced', 'Cleaned', 'Firmware update'], new Utf8()),
    cost:      vectorFromArray([50.0, 25.0, null, 0.0], new Float64()),
  });
  const inserted = await insertFromTable(conn, maintenanceDef, maintenanceData);
  log(`  Inserted ${inserted} maintenance records from Arrow Table`);

  // Verify the round-trip
  const maint = await tableFromQuery(conn, 'SELECT * FROM maintenance_log ORDER BY log_id');
  for (let i = 0; i < maint.numRows; i++) {
    const id = maint.getChild('log_id').get(i);
    const action = maint.getChild('action').get(i);
    const cost = maint.getChild('cost').get(i);
    log(`  #${id}: ${action.padEnd(20)} cost=${cost ?? 'N/A'}`);
  }

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
  log(`Pool created: min=1, max=5, idleTimeout=10s`);

  // pool.query — simple shorthand
  const countResult = await pool.query('SELECT COUNT(*) AS n FROM readings');
  log(`\npool.query: ${countResult[0].getInt64(0).toLocaleString()} readings`);

  // pool.queryParams — parameterized shorthand
  const sensorAlerts = await pool.queryParams(
    'SELECT * FROM alerts WHERE sensor_id = $1 AND acknowledged = $2',
    [1, false]
  );
  log(`pool.queryParams: ${sensorAlerts.length} unacknowledged alerts for sensor 1`);

  // pool.command — DML shorthand
  await pool.command("UPDATE alerts SET acknowledged = TRUE WHERE severity = 'info'");
  log('pool.command: acknowledged all info-level alerts');

  // pool.use — auto acquire/release
  const avgTemp = await pool.use(async (c) => {
    const rows = await c.executeQuery(
      'SELECT AVG(temperature) FROM readings WHERE sensor_id = 1'
    );
    return rows[0].getFloat64(0);
  });
  log(`pool.use: avg temp for sensor 1 = ${avgTemp.toFixed(2)}°C`);

  // Concurrent pool usage
  log('\nConcurrent queries (3 parallel):');
  const t0 = performance.now();
  const [r1, r2, r3] = await Promise.all([
    pool.query('SELECT COUNT(*) FROM readings WHERE sensor_id = 1'),
    pool.query('SELECT COUNT(*) FROM readings WHERE sensor_id = 2'),
    pool.query('SELECT COUNT(*) FROM readings WHERE sensor_id = 3'),
  ]);
  const concurrentMs = performance.now() - t0;
  log(`  Sensor 1: ${r1[0].getInt64(0).toLocaleString()} readings`);
  log(`  Sensor 2: ${r2[0].getInt64(0).toLocaleString()} readings`);
  log(`  Sensor 3: ${r3[0].getInt64(0).toLocaleString()} readings`);
  log(`  Completed in ${concurrentMs.toFixed(0)}ms`);

  // Pool stats
  log(`\nPool stats: size=${pool.size}, idle=${pool.idle}, active=${pool.active}, pending=${pool.pending}`);

  await pool.close();
  log('Pool closed ✓');

  // ═══════════════════════════════════════════════════════════════════════════
  // 16. Error handling
  // ═══════════════════════════════════════════════════════════════════════════

  section('Error handling');

  // Query errors
  try {
    await conn.executeQuery('SELECT * FROM nonexistent_table');
  } catch (err) {
    log(`Query error caught: "${err.message.split('\n')[0]}"`);
  }

  // Reconnect after error (errors can leave the TCP connection in a dirty state)
  await conn.close();
  conn = await Connection.connect(hyper.endpoint, dbPath, CreateMode.DoNotCreate);
  catalog = new Catalog(conn);

  // Pool after close
  try {
    await pool.query('SELECT 1');
  } catch (err) {
    log(`Pool-after-close error: "${err.message}"`);
  }

  // ═══════════════════════════════════════════════════════════════════════════
  // Cleanup
  // ═══════════════════════════════════════════════════════════════════════════

  section('Cleanup');

  await catalog.dropTable('maintenance_log');
  await catalog.dropTable('alerts');
  await catalog.dropTable('readings');
  await catalog.dropTable('sensors');
  await catalog.dropSchema('monitoring', true);
  log('Dropped all tables and schemas');

  await conn.close();
  log('Connection closed');

  hyper.close();
  log('Hyper server stopped');

  console.log('\n╔══════════════════════════════════════════════════════════════════════╗');
  console.log('║  ✓ Complete API tour finished successfully!                        ║');
  console.log('╚══════════════════════════════════════════════════════════════════════╝\n');
}

main().catch((err) => {
  console.error('\nExample failed:', err);
  process.exit(1);
});
