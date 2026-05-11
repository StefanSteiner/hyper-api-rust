// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

/**
 * SQL generation for creating demo databases with configurable distributions.
 */

export const SUPPORTED_TYPES = [
  'INT', 'BIGINT', 'SMALLINT', 'DOUBLE PRECISION', 'TEXT', 'BOOLEAN', 'DATE', 'TIMESTAMP',
] as const;

export type SqlTypeName = (typeof SUPPORTED_TYPES)[number];

// Distribution types available for each SQL type
export const DISTRIBUTIONS_BY_TYPE: Record<SqlTypeName, string[]> = {
  'INT':              ['sequential', 'uniform', 'normal', 'exponential'],
  'BIGINT':           ['sequential', 'uniform', 'normal', 'exponential'],
  'SMALLINT':         ['sequential', 'uniform', 'normal'],
  'DOUBLE PRECISION': ['uniform', 'normal', 'lognormal', 'exponential', 'bimodal'],
  'TEXT':             ['categorical', 'uuid', 'words'],
  'BOOLEAN':          ['bernoulli'],
  'DATE':             ['uniform_range'],
  'TIMESTAMP':        ['uniform_range'],
};

export interface ColumnSpec {
  name: string;
  type: SqlTypeName;
  nullable: boolean;
  nullPercent: number; // 0-100
  distribution: string;
  params: Record<string, any>;
}

export interface TableSpec {
  name: string;
  rowCount: number;
  columns: ColumnSpec[];
}

export interface GenerateSpec {
  dbPath: string;
  tables: TableSpec[];
}

function quoteIdent(name: string): string {
  return `"${name.replace(/"/g, '""')}"`;
}

// Box-Muller normal SQL expression
const NORMAL_SQL = (mean: number, stddev: number) =>
  `(${mean} + SQRT(-2.0 * LN(GREATEST(random(), 1e-10))) * COS(2.0 * 3.141592653589793 * random()) * ${stddev})`;

/**
 * Generate the SQL expression for a single column value.
 */
function columnValueExpr(col: ColumnSpec, rowAlias: string): string {
  const p = col.params || {};
  let expr: string;

  switch (col.type) {
    case 'INT':
    case 'BIGINT':
    case 'SMALLINT': {
      const castType = col.type === 'BIGINT' ? 'BIGINT' : col.type === 'SMALLINT' ? 'SMALLINT' : 'INT';
      switch (col.distribution) {
        case 'sequential':
          expr = `CAST(${rowAlias} + ${p.start ?? 0} AS ${castType})`;
          break;
        case 'uniform':
          expr = `CAST(${p.min ?? 1} + FLOOR(random() * ${(p.max ?? 1000) - (p.min ?? 1) + 1}) AS ${castType})`;
          break;
        case 'normal':
          expr = `CAST(ROUND(${NORMAL_SQL(p.mean ?? 500, p.stddev ?? 100)}) AS ${castType})`;
          break;
        case 'exponential':
          expr = `CAST(FLOOR(-LN(GREATEST(1.0 - random(), 1e-10)) / ${p.lambda ?? 0.01}) AS ${castType})`;
          break;
        default:
          expr = `CAST(${rowAlias} AS ${castType})`;
      }
      break;
    }

    case 'DOUBLE PRECISION': {
      switch (col.distribution) {
        case 'uniform':
          expr = `ROUND(CAST(${p.min ?? 0} + random() * ${(p.max ?? 100) - (p.min ?? 0)} AS DOUBLE PRECISION), 2)`;
          break;
        case 'normal':
          expr = `ROUND(CAST(${NORMAL_SQL(p.mean ?? 0, p.stddev ?? 1)} AS DOUBLE PRECISION), 2)`;
          break;
        case 'lognormal':
          expr = `ROUND(CAST(LEAST(${p.cap ?? 50000}, EXP(${NORMAL_SQL(p.mu ?? 3.5, p.sigma ?? 1.0)})) AS DOUBLE PRECISION), 2)`;
          break;
        case 'exponential':
          expr = `ROUND(CAST(${p.offset ?? 0} + (-LN(GREATEST(1.0 - random(), 1e-10)) / ${p.lambda ?? 0.01}) AS DOUBLE PRECISION), 2)`;
          break;
        case 'bimodal': {
          const m1 = p.mean1 ?? 100, s1 = p.stddev1 ?? 10;
          const m2 = p.mean2 ?? 200, s2 = p.stddev2 ?? 15;
          const w = p.weight ?? 0.6;
          expr = `ROUND(CAST(CASE WHEN random() < ${w} THEN ${NORMAL_SQL(m1, s1)} ELSE ${NORMAL_SQL(m2, s2)} END AS DOUBLE PRECISION), 2)`;
          break;
        }
        default:
          expr = `ROUND(CAST(random() * 100 AS DOUBLE PRECISION), 2)`;
      }
      break;
    }

    case 'TEXT': {
      switch (col.distribution) {
        case 'categorical': {
          const values: string[] = p.values ?? ['A', 'B', 'C', 'D', 'E'];
          const n = values.length;
          // Build CASE with equal or weighted random
          const arrayLit = `ARRAY[${values.map((v: string) => `'${v.replace(/'/g, "''")}'`).join(',')}]`;
          if (p.zipf) {
            expr = `(${arrayLit})[CAST(1 + FLOOR(POWER(random(), ${p.zipfExponent ?? 1.5}) * ${n}) AS INT)]`;
          } else {
            expr = `(${arrayLit})[CAST(1 + FLOOR(random() * ${n}) AS INT)]`;
          }
          break;
        }
        case 'uuid':
          // Pseudo-UUID from random hex
          expr = `CAST(
            LPAD(CAST(CAST(FLOOR(random() * 2147483647) AS INT) AS TEXT), 8, '0') || '-'
            || LPAD(CAST(CAST(FLOOR(random() * 65535) AS INT) AS TEXT), 4, '0') || '-'
            || LPAD(CAST(CAST(FLOOR(random() * 65535) AS INT) AS TEXT), 4, '0') || '-'
            || LPAD(CAST(CAST(FLOOR(random() * 65535) AS INT) AS TEXT), 4, '0') || '-'
            || LPAD(CAST(CAST(FLOOR(random() * 2147483647) AS INT) AS TEXT), 12, '0')
          AS TEXT)`;
          break;
        case 'words': {
          const words: string[] = p.words ?? [
            'alpha', 'beta', 'gamma', 'delta', 'epsilon', 'zeta', 'eta', 'theta',
            'iota', 'kappa', 'lambda', 'mu', 'nu', 'xi', 'omicron', 'pi',
          ];
          const n = words.length;
          const arrayLit = `ARRAY[${words.map((w: string) => `'${w.replace(/'/g, "''")}'`).join(',')}]`;
          const wordCount = p.wordCount ?? 3;
          const parts = Array.from({ length: wordCount }, () =>
            `(${arrayLit})[CAST(1 + FLOOR(random() * ${n}) AS INT)]`
          );
          expr = parts.join(` || ' ' || `);
          break;
        }
        default:
          expr = `'value_' || CAST(${rowAlias} AS TEXT)`;
      }
      break;
    }

    case 'BOOLEAN':
      expr = `random() < ${(p.trueProbability ?? 50) / 100.0}`;
      break;

    case 'DATE': {
      const startDate = p.start ?? '2020-01-01';
      const endDate = p.end ?? '2025-12-31';
      // Compute approx days between
      const daySpan = p.daySpan ?? 2190;
      expr = `DATE '${startDate}' + CAST(FLOOR(random() * ${daySpan}) AS INT)`;
      break;
    }

    case 'TIMESTAMP': {
      const startTs = p.start ?? '2020-01-01 00:00:00';
      const secondSpan = p.secondSpan ?? 189216000; // ~6 years in seconds
      expr = `TIMESTAMP '${startTs}' + CAST(FLOOR(random() * ${secondSpan}) AS INT) * INTERVAL '1 second'`;
      break;
    }

    default:
      expr = `NULL`;
  }

  // Wrap with nullable logic
  if (col.nullable && col.nullPercent > 0) {
    return `CASE WHEN random() < ${col.nullPercent / 100.0} THEN NULL ELSE ${expr} END`;
  }

  return expr;
}

/**
 * Generate CREATE TABLE SQL for a table spec.
 */
export function buildCreateTableSql(table: TableSpec): string {
  const cols = table.columns.map((col) => {
    const nullStr = col.nullable ? '' : ' NOT NULL';
    return `  ${quoteIdent(col.name)} ${col.type}${nullStr}`;
  });
  return `CREATE TABLE ${quoteIdent(table.name)} (\n${cols.join(',\n')}\n)`;
}

/**
 * Generate INSERT INTO ... SELECT SQL for populating a table.
 */
export function buildInsertSql(table: TableSpec): string {
  const colExprs = table.columns.map((col) => {
    const expr = columnValueExpr(col, 'i');
    return `  ${expr} AS ${quoteIdent(col.name)}`;
  });

  return `INSERT INTO ${quoteIdent(table.name)}\nSELECT\n${colExprs.join(',\n')}\nFROM generate_series(1, ${table.rowCount}) AS s(i)`;
}
