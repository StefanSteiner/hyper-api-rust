// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

const BASE = '/api';

export interface ColumnInfo {
  name: string;
  typeName: string;
}

export interface SchemaTable {
  name: string;
  columns: ColumnInfo[];
}

export interface SchemaNode {
  schema: string;
  tables: SchemaTable[];
}

export interface OpenResult {
  database: string;
  schemas: SchemaNode[];
}

export interface PreviewResult {
  columns: ColumnInfo[];
  rows: Record<string, string | null>[];
  rowCount: number;
  /** Present only on the initial load (withCount=true). */
  totalRowCount: number | null;
  offset: number;
}

export interface ColumnStat {
  name: string;
  typeName: string;
  rowCount: number;
  nullCount: number;
  nullPercent: number;
  distinctCount: number;
  min?: number | string | null;
  max?: number | string | null;
  mean?: number | null;
  stddev?: number | null;
  cv?: number | null;
  minLength?: number | null;
  maxLength?: number | null;
  avgLength?: number | null;
  topValues?: { value: string; count: number }[];
  trueCount?: number;
  falseCount?: number;
  truePercent?: number;
}

export interface StatsResult {
  stats: ColumnStat[];
}

export interface PreExecutionStats {
  parsingTimeS?: number | null;
  compilationTimeS?: number | null;
  elapsedS?: number | null;
  peakMemoryMb?: number | null;
}

export interface ExecutionStats {
  elapsedS?: number | null;
  cpuTimeS?: number | null;
  threadTimeS?: number | null;
  waitTimeS?: number | null;
  processedRowsTotal?: number | null;
  processedRowsNative?: number | null;
  storageAccessTimeS?: number | null;
  storageAccessCount?: number | null;
  storageAccessBytes?: number | null;
  peakMemoryMb?: number | null;
}

export interface QueryStats {
  elapsedS: number;
  commitTimeS?: number | null;
  timeToScheduleS?: number | null;
  preExecution?: PreExecutionStats | null;
  execution?: ExecutionStats | null;
  resultSizeMb?: number | null;
  peakResultBufferMemoryMb?: number | null;
  planCacheStatus?: string | null;
  planCacheHitCount?: number | null;
  statementType?: string | null;
  rows?: number | null;
  cols?: number | null;
  queryTruncated?: string | null;
}

export interface QueryResultSelect {
  type: 'query';
  columns: ColumnInfo[];
  rows: Record<string, string | null>[];
  rowCount: number;
  durationMs: number;
  queryStats?: QueryStats;
}

export interface QueryResultCommand {
  type: 'command';
  rowsAffected: number;
  durationMs: number;
  queryStats?: QueryStats;
}

export type QueryResult = QueryResultSelect | QueryResultCommand;

export interface TrackedQuery {
  sql: string;
  rowCount: number | null;
  durationMs: number;
  source: string;
  connectionId?: number;
  queryStats?: QueryStats;
}

let _onInternalQueries: ((queries: TrackedQuery[]) => void) | null = null;

export function setOnInternalQueries(cb: ((queries: TrackedQuery[]) => void) | null) {
  _onInternalQueries = cb;
}

async function request<T>(url: string, options?: RequestInit): Promise<T> {
  const res = await fetch(BASE + url, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  const data = await res.json();
  if (!res.ok) throw new Error(data.error || `HTTP ${res.status}`);
  if (data._queries && _onInternalQueries) {
    _onInternalQueries(data._queries);
  }
  return data as T;
}

export function openDatabase(path: string): Promise<OpenResult> {
  return request('/open', {
    method: 'POST',
    body: JSON.stringify({ path }),
  });
}

export function closeDatabase(path: string): Promise<{ ok: boolean }> {
  return request('/close', {
    method: 'POST',
    body: JSON.stringify({ path }),
  });
}

export function getPreview(
  db: string,
  schema: string,
  table: string,
  limit = 100,
  offset = 0,
  sortColumn?: string,
  sortDir?: 'asc' | 'desc',
  withCount = false,
): Promise<PreviewResult> {
  const params = new URLSearchParams({ db, limit: String(limit), offset: String(offset) });
  if (sortColumn) params.set('sortColumn', sortColumn);
  if (sortDir) params.set('sortDir', sortDir);
  if (withCount) params.set('withCount', '1');
  return request(`/preview/${encodeURIComponent(schema)}/${encodeURIComponent(table)}?${params}`);
}

export function getStats(db: string, schema: string, table: string): Promise<StatsResult> {
  const params = new URLSearchParams({ db });
  return request(`/stats/${encodeURIComponent(schema)}/${encodeURIComponent(table)}?${params}`);
}

export function executeQuery(db: string, sql: string): Promise<QueryResult> {
  return request('/query', {
    method: 'POST',
    body: JSON.stringify({ db, sql }),
  });
}

export interface BrowseItem {
  name: string;
  path: string;
  isDir: boolean;
  isHyper: boolean;
  size: number | null;
  lastModified: string | null;
}

export interface BrowseResult {
  dir: string;
  items: BrowseItem[];
}

export function browseDir(dir?: string): Promise<BrowseResult> {
  const params = dir ? new URLSearchParams({ dir }) : '';
  return request(`/browse${params ? '?' + params : ''}`);
}

export interface HistogramBucket {
  lo: number;
  hi: number;
  count: number;
}

export interface FFTBin {
  frequency: number;
  magnitude: number;
  raw: number;
}

export interface FourierSeriesTerm {
  n: number;
  an: number;
  bn: number;
  amplitude: number;
}

export interface FourierSeriesData {
  a0: number;
  terms: FourierSeriesTerm[];
  reconstructed: number[];
}

export interface ColumnDetail {
  name: string;
  typeName: string;
  rowCount: number;
  nullCount: number;
  nullPercent: number;
  distinctCount: number;
  cardinality: number;
  // Numeric
  min?: number | string | null;
  max?: number | string | null;
  mean?: number | null;
  stddev?: number | null;
  cv?: number | null;
  variance?: number | null;
  sum?: number | null;
  median?: number | null;
  p10?: number | null;
  p25?: number | null;
  p75?: number | null;
  p90?: number | null;
  histogram?: HistogramBucket[];
  fft?: FFTBin[];
  fourierSeries?: FourierSeriesData | null;
  // Text
  minLength?: number | null;
  maxLength?: number | null;
  avgLength?: number | null;
  // Bool
  trueCount?: number;
  falseCount?: number;
  truePercent?: number;
  // All types
  topValues?: { value: string; count: number }[];
}

export interface ColumnDetailResult {
  detail: ColumnDetail | null;
}

export function getColumnDetail(
  db: string,
  schema: string,
  table: string,
  column: string,
): Promise<ColumnDetailResult> {
  const params = new URLSearchParams({ db });
  return request(
    `/column-detail/${encodeURIComponent(schema)}/${encodeURIComponent(table)}/${encodeURIComponent(column)}?${params}`,
  );
}

// =============================================================================
// Database Generator
// =============================================================================

export interface GenerateColumnSpec {
  name: string;
  type: string;
  nullable: boolean;
  nullPercent: number;
  distribution: string;
  params: Record<string, any>;
}

export interface GenerateTableSpec {
  name: string;
  rowCount: number;
  columns: GenerateColumnSpec[];
}

export interface GenerateSpec {
  dbPath: string;
  tables: GenerateTableSpec[];
}

export interface GenerateMetaResult {
  types: string[];
  distributionsByType: Record<string, string[]>;
}

export interface GenerateTableResult {
  table: string;
  rowCount: number;
  durationMs: number;
}

export interface GenerateResult {
  dbPath: string;
  results: GenerateTableResult[];
}

export function getGenerateMeta(): Promise<GenerateMetaResult> {
  return request('/generate-meta');
}

export function generateDatabase(spec: GenerateSpec): Promise<GenerateResult> {
  return request('/generate', {
    method: 'POST',
    body: JSON.stringify(spec),
  });
}
