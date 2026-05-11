/**
 * Apache Arrow integration for hyperdb-api-node.
 *
 * Convenience functions that bridge hyperdb-api-node's raw Arrow IPC buffers
 * with the `apache-arrow` JS package. Requires `apache-arrow` as a peer
 * dependency (`npm install apache-arrow`).
 *
 * @module hyperdb-api-node/arrow
 *
 * @example
 * ```js
 * import { Connection, HyperProcess, CreateMode } from 'hyperdb-api-node';
 * import { tableFromQuery, insertFromTable } from 'hyperdb-api-node/arrow.mjs';
 * import { tableFromIPC, vectorFromArray, makeTable } from 'apache-arrow';
 * ```
 */

import { tableFromIPC, tableToIPC } from 'apache-arrow';
import { createRequire } from 'module';
const require = createRequire(import.meta.url);

// =============================================================================
// Query → Arrow Table
// =============================================================================

/**
 * Executes a SQL query and returns an Apache Arrow Table.
 *
 * This is the primary way to get structured columnar data out of Hyper
 * and into the Arrow JS ecosystem (Observable, Arquero, DuckDB-WASM, etc.).
 *
 * @param {import('./index.js').Connection} conn - The database connection.
 * @param {string} sql - The SQL SELECT query.
 * @returns {Promise<import('apache-arrow').Table>} An Arrow Table.
 *
 * @example
 * ```js
 * const table = await tableFromQuery(conn, 'SELECT * FROM sales');
 * console.log(table.schema.fields.map(f => f.name));
 * console.log(table.numRows);
 *
 * // Access columns as typed arrays
 * const revenue = table.getChild('revenue').toArray(); // Float64Array
 * ```
 */
export async function tableFromQuery(conn, sql) {
  const buf = await conn.executeQueryToArrow(sql);
  return tableFromIPC(buf);
}

/**
 * Exports an entire table as an Apache Arrow Table.
 *
 * @param {import('./index.js').Connection} conn - The database connection.
 * @param {string} tableName - The table to export.
 * @returns {Promise<import('apache-arrow').Table>} An Arrow Table.
 *
 * @example
 * ```js
 * const table = await exportTable(conn, 'measurements');
 * for (const batch of table.batches) {
 *   console.log(`Batch: ${batch.numRows} rows`);
 * }
 * ```
 */
export async function exportTable(conn, tableName) {
  const buf = await conn.exportTableToArrow(tableName);
  return tableFromIPC(buf);
}

// =============================================================================
// Query → RecordBatch[]
// =============================================================================

/**
 * Executes a SQL query and returns an array of Arrow RecordBatches.
 *
 * Useful when you need batch-level control (e.g., streaming to a file
 * or forwarding to another Arrow consumer).
 *
 * @param {import('./index.js').Connection} conn - The database connection.
 * @param {string} sql - The SQL SELECT query.
 * @returns {Promise<import('apache-arrow').RecordBatch[]>} Array of RecordBatches.
 *
 * @example
 * ```js
 * const batches = await batchesFromQuery(conn, 'SELECT * FROM events');
 * for (const batch of batches) {
 *   const ids = batch.getChild('id').toArray();
 *   console.log(`Batch of ${ids.length} rows`);
 * }
 * ```
 */
export async function batchesFromQuery(conn, sql) {
  const table = await tableFromQuery(conn, sql);
  return table.batches;
}

// =============================================================================
// Query → Arrow IPC file bytes
// =============================================================================

/**
 * Executes a SQL query and returns Arrow IPC file-format bytes (.arrow).
 *
 * The result includes a file footer for random access, unlike the stream
 * format returned by `executeQueryToArrow()`. Use this when writing `.arrow`
 * files that will be read by other tools (DuckDB, Polars, pandas, etc.).
 *
 * @param {import('./index.js').Connection} conn - The database connection.
 * @param {string} sql - The SQL SELECT query.
 * @returns {Promise<Uint8Array>} Arrow IPC file-format bytes.
 *
 * @example
 * ```js
 * import { writeFileSync } from 'fs';
 * const bytes = await queryToArrowFile(conn, 'SELECT * FROM sales');
 * writeFileSync('sales.arrow', bytes);
 * ```
 */
export async function queryToArrowFile(conn, sql) {
  const table = await tableFromQuery(conn, sql);
  return tableToIPC(table, 'file');
}

// =============================================================================
// Arrow Table → Hyper insert
// =============================================================================

/**
 * Inserts data from an Apache Arrow Table into a Hyper table.
 *
 * Extracts columns from the Arrow Table and inserts them in batches
 * using the COPY protocol. Supports Int, BigInt, Float/Double, and Utf8
 * column types.
 *
 * @param {import('./index.js').Connection} conn - The database connection.
 * @param {import('./index.js').TableDefinition} tableDef - Target table definition.
 * @param {import('apache-arrow').Table} arrowTable - The Arrow Table to insert.
 * @param {object} [options] - Insert options.
 * @param {number} [options.batchSize=50000] - Rows per insert batch.
 * @returns {Promise<number>} Total number of rows inserted.
 *
 * @example
 * ```js
 * import { vectorFromArray, makeTable, Float64, Int32, Utf8 } from 'apache-arrow';
 *
 * const arrowTable = makeTable({
 *   id: vectorFromArray([1, 2, 3], new Int32()),
 *   name: vectorFromArray(['Alice', 'Bob', 'Charlie'], new Utf8()),
 *   score: vectorFromArray([95.5, 88.0, null], new Float64()),
 * });
 *
 * const count = await insertFromTable(conn, tableDef, arrowTable);
 * console.log(`Inserted ${count} rows`);
 * ```
 */
export async function insertFromTable(conn, tableDef, arrowTable, _options = {}) {
  // Fast path — serialize the Arrow table to IPC bytes and stream it
  // directly into Hyper via ArrowInserter. No per-row JS↔Rust conversion.
  const { ArrowInserter } = require('./index.js');
  const { tableToIPC } = await import('apache-arrow');
  const inserter = ArrowInserter.create(conn, tableDef);
  await inserter.insertRaw(Buffer.from(tableToIPC(arrowTable, 'stream')));
  return await inserter.execute();
}

// =============================================================================
// Schema utilities
// =============================================================================

/**
 * Returns the Arrow schema for a Hyper query without fetching data.
 *
 * Executes the query with `LIMIT 0` to get only the schema.
 *
 * @param {import('./index.js').Connection} conn - The database connection.
 * @param {string} sql - The SQL SELECT query.
 * @returns {Promise<import('apache-arrow').Schema>} The Arrow Schema.
 *
 * @example
 * ```js
 * const schema = await querySchema(conn, 'SELECT * FROM measurements');
 * for (const field of schema.fields) {
 *   console.log(`${field.name}: ${field.type}`);
 * }
 * ```
 */
export async function querySchema(conn, sql) {
  const wrapped = `SELECT * FROM (${sql}) AS _q LIMIT 0`;
  const table = await tableFromQuery(conn, wrapped);
  return table.schema;
}
