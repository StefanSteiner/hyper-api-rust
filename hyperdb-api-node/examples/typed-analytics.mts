#!/usr/bin/env npx tsx
/**
 * Typed Analytics Example (TypeScript)
 *
 * Demonstrates hyperdb-api-node with full TypeScript type safety.
 * Builds a small analytics pipeline: create → insert → query → aggregate → Arrow export.
 *
 * Run:
 *   HYPERD_PATH=/path/to/hyperd npx tsx examples/typed-analytics.mts
 */

import { existsSync, mkdirSync, writeFileSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';
import { tableFromIPC } from 'apache-arrow';
import { createRequire } from 'module';

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
} = require('../index.js') as typeof import('../index.js');

import type {
  RowData,
  ColumnarChunk,
  ResultColumnInfo,
} from '../index.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const OUT_DIR = join(__dirname, '..', 'test_results');
if (!existsSync(OUT_DIR)) mkdirSync(OUT_DIR, { recursive: true });

// ── Helper types for our domain ─────────────────────────────────────────────

interface SensorReading {
  sensorId: number;
  temperature: number;
  humidity: number;
  timestamp: number;
}

interface SensorStats {
  sensorId: number;
  avgTemp: number;
  avgHumidity: number;
  readingCount: number;
}

// ── Type-safe row extraction ────────────────────────────────────────────────

function rowToReading(row: RowData): SensorReading {
  return {
    sensorId: row.getInt32(0)!,
    temperature: row.getFloat64(1)!,
    humidity: row.getFloat64(2)!,
    timestamp: row.getInt64(3)!,
  };
}

function rowToStats(row: RowData): SensorStats {
  return {
    sensorId: row.getInt32(0)!,
    avgTemp: row.getFloat64(1)!,
    avgHumidity: row.getFloat64(2)!,
    readingCount: row.getInt64(3)! as number,
  };
}

// ── Main ────────────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  console.log('Typed Analytics Example (TypeScript)\n');

  // 1. Start Hyper and connect
  const hyper = new HyperProcess();
  const dbPath: string = join(OUT_DIR, 'typed_example.hyper');

  const conn: InstanceType<typeof Connection> = await Connection.connect(
    hyper.endpoint,
    dbPath,
    CreateMode.CreateAndReplace,
  );
  console.log(`Connected: ${conn.database}`);

  // 2. Define schema with full type safety
  const readingsDef: InstanceType<typeof TableDefinition> = new TableDefinition('readings');
  readingsDef.addColumn('sensor_id', SqlType.int(), false);
  readingsDef.addColumn('temperature', SqlType.double(), false);
  readingsDef.addColumn('humidity', SqlType.double(), false);
  readingsDef.addColumn('recorded_at', SqlType.bigInt(), false);

  const catalog: InstanceType<typeof Catalog> = new Catalog(conn);
  await catalog.createTable(readingsDef);
  console.log(`Created table: ${readingsDef.name} (${readingsDef.columnCount} columns)`);

  // 3. Generate and insert typed data
  const readings: SensorReading[] = [];
  for (let i = 0; i < 50_000; i++) {
    readings.push({
      sensorId: (i % 5) + 1,
      temperature: 18 + Math.sin(i * 0.01) * 8,
      humidity: 40 + Math.cos(i * 0.02) * 20,
      timestamp: 1700000000000 + i * 60_000,
    });
  }

  const inserter: InstanceType<typeof RowInserter> = new RowInserter(conn, readingsDef);
  const rows = readings.map((r): [number, number, number, number] => [
    r.sensorId,
    r.temperature,
    r.humidity,
    r.timestamp,
  ]);
  inserter.addRows(rows);
  const inserted: number = await inserter.execute();
  console.log(`Inserted ${inserted.toLocaleString()} readings\n`);

  // 4. Query with typed results
  console.log('── Recent readings (typed) ──');
  const recent: RowData[] = await conn.executeQuery(
    'SELECT * FROM readings ORDER BY recorded_at DESC LIMIT 5'
  );
  const typedRecent: SensorReading[] = recent.map(rowToReading);
  for (const r of typedRecent) {
    console.log(
      `  sensor=${r.sensorId}  temp=${r.temperature.toFixed(1)}°C  ` +
      `humidity=${r.humidity.toFixed(1)}%  ts=${r.timestamp}`
    );
  }

  // 5. Prepared statement (type-safe params, server-side binding)
  console.log('\n── Sensor 3 readings > 24°C (prepared) ──');
  const hotStmt = await conn.prepare(
    'SELECT * FROM readings WHERE sensor_id = $1 AND temperature > $2 ORDER BY temperature DESC LIMIT 5'
  );
  const hot: RowData[] = await hotStmt.query([3, 24.0]);
  await hotStmt.close();
  for (const row of hot) {
    const r = rowToReading(row);
    console.log(`  temp=${r.temperature.toFixed(2)}°C  humidity=${r.humidity.toFixed(1)}%`);
  }

  // 6. Aggregation with typed results
  console.log('\n── Sensor statistics ──');
  const statsRows: RowData[] = await conn.executeQuery(`
    SELECT sensor_id, AVG(temperature), AVG(humidity), COUNT(*)
    FROM readings GROUP BY sensor_id ORDER BY sensor_id
  `);
  const stats: SensorStats[] = statsRows.map(rowToStats);
  console.log('  ID   Avg Temp    Avg Hum    Readings');
  console.log('  ' + '─'.repeat(45));
  for (const s of stats) {
    console.log(
      `  ${String(s.sensorId).padStart(2)}   ` +
      `${s.avgTemp.toFixed(2).padStart(8)}°C  ` +
      `${s.avgHumidity.toFixed(2).padStart(7)}%  ` +
      `${s.readingCount.toLocaleString().padStart(8)}`
    );
  }

  // 7. Columnar stream with typed chunk access
  console.log('\n── Columnar scan ──');
  const colStream = conn.executeQueryColumnar('SELECT * FROM readings');
  let totalTemp = 0;
  let count = 0;
  let chunk: ColumnarChunk | null;
  while ((chunk = await colStream.nextChunk()) !== null) {
    const temps: number[] = chunk.getFloat64Column(1);
    for (let i = 0; i < temps.length; i++) totalTemp += temps[i];
    count += chunk.rowCount;
  }
  console.log(`  Scanned ${count.toLocaleString()} rows, avg temp = ${(totalTemp / count).toFixed(2)}°C`);

  // 8. Schema introspection (typed)
  console.log('\n── Schema ──');
  const schema: ResultColumnInfo[] = await conn.querySchema('SELECT * FROM readings');
  for (const col of schema) {
    console.log(`  [${col.index}] ${col.name.padEnd(14)} ${col.typeName}`);
  }

  // 9. Arrow export (typed Buffer)
  console.log('\n── Arrow export ──');
  const arrowBuf: Buffer = await conn.executeQueryToArrow(
    'SELECT sensor_id, temperature, humidity FROM readings'
  );
  const arrowTable = tableFromIPC(arrowBuf);
  console.log(`  Arrow Table: ${arrowTable.numRows} rows, ${arrowTable.numCols} columns`);
  console.log(`  Schema: ${arrowTable.schema.fields.map(f => `${f.name}:${f.type}`).join(', ')}`);

  // Write .arrow file
  const arrowPath = join(OUT_DIR, 'typed_readings.arrows');
  writeFileSync(arrowPath, arrowBuf);
  console.log(`  Wrote ${arrowPath} (${(arrowBuf.byteLength / 1024).toFixed(1)} KB)`);

  // 10. ConnectionBuilder (typed)
  console.log('\n── ConnectionBuilder ──');
  const conn2: InstanceType<typeof Connection> = await new ConnectionBuilder(hyper.endpoint)
    .database(join(OUT_DIR, 'typed_builder.hyper'))
    .createMode(CreateMode.CreateIfNotExists)
    .loginTimeout(5_000)
    .build();
  console.log(`  Builder connection: ${conn2.database}`);
  await conn2.close();

  // Cleanup
  await conn.close();
  hyper.close();

  console.log('\n✓ TypeScript example completed successfully.\n');
}

main().catch((err: Error) => {
  console.error('Example failed:', err);
  process.exit(1);
});
