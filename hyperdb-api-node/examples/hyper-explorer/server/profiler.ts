// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

/**
 * Generates SQL for per-column statistics based on column type.
 */

export interface ColumnMeta {
  name: string;
  typeName: string;
  index: number;
}

export interface ColumnStat {
  name: string;
  typeName: string;
  rowCount: number;
  nullCount: number;
  nullPercent: number;
  distinctCount: number;
  // Numeric
  min?: number | string | null;
  max?: number | string | null;
  mean?: number | null;
  stddev?: number | null;
  cv?: number | null;
  // Text
  minLength?: number | null;
  maxLength?: number | null;
  avgLength?: number | null;
  topValues?: { value: string; count: number }[];
  // Bool
  trueCount?: number;
  falseCount?: number;
  truePercent?: number;
}

export function isNumericType(typeName: string): boolean {
  const t = typeName.toUpperCase();
  return (
    t.includes('INT') ||
    t.includes('DOUBLE') ||
    t.includes('FLOAT') ||
    t.includes('NUMERIC') ||
    t.includes('DECIMAL') ||
    t === 'SMALLINT' ||
    t === 'BIGINT'
  );
}

export function isTextType(typeName: string): boolean {
  const t = typeName.toUpperCase();
  return t.includes('TEXT') || t.includes('VARCHAR') || t.includes('CHAR');
}

export function isBoolType(typeName: string): boolean {
  return typeName.toUpperCase() === 'BOOL' || typeName.toUpperCase() === 'BOOLEAN';
}

export function isDateType(typeName: string): boolean {
  const t = typeName.toUpperCase();
  return t.includes('DATE') || t.includes('TIMESTAMP');
}

function quoteIdent(name: string): string {
  return `"${name.replace(/"/g, '""')}"`;
}

/**
 * Build SQL for detailed single-column stats (extends basic stats).
 * Includes median, p25, p75, cardinality for all; sum/variance for numeric.
 */
export function buildColumnDetailQuery(
  schema: string,
  table: string,
  columnName: string,
  typeName: string,
): string {
  const fqn = `${quoteIdent(schema)}.${quoteIdent(table)}`;
  const q = quoteIdent(columnName);
  const parts: string[] = [
    `COUNT(*) AS "rowCount"`,
    `COUNT(*) - COUNT(${q}) AS "nullCount"`,
    `COUNT(DISTINCT ${q}) AS "distinctCount"`,
  ];

  if (isNumericType(typeName)) {
    parts.push(`MIN(${q}) AS "min"`);
    parts.push(`MAX(${q}) AS "max"`);
    parts.push(`AVG(CAST(${q} AS DOUBLE PRECISION)) AS "mean"`);
    parts.push(`STDDEV_POP(CAST(${q} AS DOUBLE PRECISION)) AS "stddev"`);
    parts.push(`VAR_POP(CAST(${q} AS DOUBLE PRECISION)) AS "variance"`);
    parts.push(`SUM(CAST(${q} AS DOUBLE PRECISION)) AS "sum"`);
    parts.push(`PERCENTILE_CONT(0.25) WITHIN GROUP (ORDER BY ${q}) AS "p25"`);
    parts.push(`PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY ${q}) AS "median"`);
    parts.push(`PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY ${q}) AS "p75"`);
    parts.push(`PERCENTILE_CONT(0.10) WITHIN GROUP (ORDER BY ${q}) AS "p10"`);
    parts.push(`PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY ${q}) AS "p90"`);
  } else if (isTextType(typeName)) {
    parts.push(`MIN(LENGTH(${q})) AS "minLength"`);
    parts.push(`MAX(LENGTH(${q})) AS "maxLength"`);
    parts.push(`AVG(CAST(LENGTH(${q}) AS DOUBLE PRECISION)) AS "avgLength"`);
  } else if (isBoolType(typeName)) {
    parts.push(`COUNT(CASE WHEN ${q} = TRUE THEN 1 END) AS "trueCount"`);
    parts.push(`COUNT(CASE WHEN ${q} = FALSE THEN 1 END) AS "falseCount"`);
  } else if (isDateType(typeName)) {
    parts.push(`MIN(${q}) AS "min"`);
    parts.push(`MAX(${q}) AS "max"`);
  }

  return `SELECT ${parts.join(', ')} FROM ${fqn}`;
}

/**
 * Build SQL for a histogram of a numeric column using WIDTH_BUCKET.
 * Returns `bucketCount` rows with bucket_lo, bucket_hi, count.
 */
export function buildHistogramQuery(
  schema: string,
  table: string,
  columnName: string,
  min: number,
  max: number,
  bucketCount: number = 20,
): string {
  const fqn = `${quoteIdent(schema)}.${quoteIdent(table)}`;
  const q = quoteIdent(columnName);
  // Avoid division by zero if min == max
  if (min === max) {
    return `SELECT CAST(${min} AS DOUBLE PRECISION) AS bucket_lo, CAST(${max} AS DOUBLE PRECISION) AS bucket_hi, COUNT(*) AS cnt FROM ${fqn} WHERE ${q} IS NOT NULL`;
  }
  const width = (max - min) / bucketCount;
  return `
    WITH buckets AS (
      SELECT WIDTH_BUCKET(CAST(${q} AS DOUBLE PRECISION), ${min}, ${max + width * 0.001}, ${bucketCount}) AS bkt
      FROM ${fqn}
      WHERE ${q} IS NOT NULL
    )
    SELECT
      CAST(${min} + (bkt - 1) * ${width} AS DOUBLE PRECISION) AS bucket_lo,
      CAST(${min} + bkt * ${width} AS DOUBLE PRECISION) AS bucket_hi,
      COUNT(*) AS cnt
    FROM buckets
    GROUP BY bkt
    ORDER BY bkt
  `.trim();
}

/**
 * Build a SQL query that computes basic stats for every column of a table.
 * Returns one row with all stats packed as columns.
 */
export function buildBasicStatsQuery(
  schema: string,
  table: string,
  columns: ColumnMeta[],
): string {
  const fqn = `${quoteIdent(schema)}.${quoteIdent(table)}`;
  const parts: string[] = [`COUNT(*) AS "rowCount"`];

  for (const col of columns) {
    const q = quoteIdent(col.name);
    const prefix = col.name.replace(/"/g, '');

    // null count and distinct count for all types
    parts.push(`COUNT(*) - COUNT(${q}) AS "${prefix}__nullCount"`);
    parts.push(`COUNT(DISTINCT ${q}) AS "${prefix}__distinctCount"`);

    if (isNumericType(col.typeName)) {
      parts.push(`MIN(${q}) AS "${prefix}__min"`);
      parts.push(`MAX(${q}) AS "${prefix}__max"`);
      parts.push(`AVG(CAST(${q} AS DOUBLE PRECISION)) AS "${prefix}__mean"`);
      parts.push(`STDDEV_POP(CAST(${q} AS DOUBLE PRECISION)) AS "${prefix}__stddev"`);
    } else if (isTextType(col.typeName)) {
      parts.push(`MIN(LENGTH(${q})) AS "${prefix}__minLength"`);
      parts.push(`MAX(LENGTH(${q})) AS "${prefix}__maxLength"`);
      parts.push(`AVG(CAST(LENGTH(${q}) AS DOUBLE PRECISION)) AS "${prefix}__avgLength"`);
    } else if (isBoolType(col.typeName)) {
      parts.push(`COUNT(CASE WHEN ${q} = TRUE THEN 1 END) AS "${prefix}__trueCount"`);
      parts.push(`COUNT(CASE WHEN ${q} = FALSE THEN 1 END) AS "${prefix}__falseCount"`);
    } else if (isDateType(col.typeName)) {
      parts.push(`MIN(${q}) AS "${prefix}__min"`);
      parts.push(`MAX(${q}) AS "${prefix}__max"`);
    }
  }

  return `SELECT ${parts.join(', ')} FROM ${fqn}`;
}

/**
 * Build a SQL query to get top N most frequent values for a text column.
 */
export function buildTopValuesQuery(
  schema: string,
  table: string,
  columnName: string,
  limit: number = 5,
): string {
  const fqn = `${quoteIdent(schema)}.${quoteIdent(table)}`;
  const q = quoteIdent(columnName);
  return `SELECT ${q} AS val, COUNT(*) AS cnt FROM ${fqn} WHERE ${q} IS NOT NULL GROUP BY ${q} ORDER BY cnt DESC LIMIT ${limit}`;
}

/**
 * Parse the single-row stats result into per-column ColumnStat objects.
 */
export function parseStatsRow(
  row: Record<string, string | null>,
  columns: ColumnMeta[],
): ColumnStat[] {
  const rowCount = Number(row['rowCount'] ?? 0);

  return columns.map((col) => {
    const prefix = col.name.replace(/"/g, '');
    const nullCount = Number(row[`${prefix}__nullCount`] ?? 0);
    const distinctCount = Number(row[`${prefix}__distinctCount`] ?? 0);
    const nullPercent = rowCount > 0 ? Math.round((nullCount / rowCount) * 10000) / 100 : 0;

    const stat: ColumnStat = {
      name: col.name,
      typeName: col.typeName,
      rowCount,
      nullCount,
      nullPercent,
      distinctCount,
    };

    if (isNumericType(col.typeName)) {
      stat.min = row[`${prefix}__min`] != null ? Number(row[`${prefix}__min`]) : null;
      stat.max = row[`${prefix}__max`] != null ? Number(row[`${prefix}__max`]) : null;
      stat.mean = row[`${prefix}__mean`] != null ? Number(row[`${prefix}__mean`]) : null;
      stat.stddev = row[`${prefix}__stddev`] != null ? Number(row[`${prefix}__stddev`]) : null;
      stat.cv = (stat.mean != null && stat.mean !== 0 && stat.stddev != null)
        ? Math.round((stat.stddev / Math.abs(stat.mean)) * 10000) / 100
        : null;
    } else if (isTextType(col.typeName)) {
      stat.minLength = row[`${prefix}__minLength`] != null ? Number(row[`${prefix}__minLength`]) : null;
      stat.maxLength = row[`${prefix}__maxLength`] != null ? Number(row[`${prefix}__maxLength`]) : null;
      stat.avgLength = row[`${prefix}__avgLength`] != null ? Number(row[`${prefix}__avgLength`]) : null;
    } else if (isBoolType(col.typeName)) {
      stat.trueCount = Number(row[`${prefix}__trueCount`] ?? 0);
      stat.falseCount = Number(row[`${prefix}__falseCount`] ?? 0);
      const nonNull = rowCount - nullCount;
      stat.truePercent = nonNull > 0 ? Math.round((stat.trueCount / nonNull) * 10000) / 100 : 0;
    } else if (isDateType(col.typeName)) {
      stat.min = row[`${prefix}__min`] ?? null;
      stat.max = row[`${prefix}__max`] ?? null;
    }

    return stat;
  });
}
