import { useState, useEffect, useCallback } from 'react';
import {
  getGenerateMeta,
  generateDatabase,
  type GenerateMetaResult,
  type GenerateTableSpec,
  type GenerateColumnSpec,
  type GenerateResult,
} from '../api';

interface Props {
  onGenerated: (dbPath: string) => void;
}

// Default distribution params per type+distribution
const DEFAULT_PARAMS: Record<string, Record<string, Record<string, any>>> = {
  'INT': {
    sequential: { start: 1 },
    uniform: { min: 1, max: 10000 },
    normal: { mean: 500, stddev: 150 },
    exponential: { lambda: 0.01 },
  },
  'BIGINT': {
    sequential: { start: 1 },
    uniform: { min: 1, max: 1000000 },
    normal: { mean: 500000, stddev: 150000 },
    exponential: { lambda: 0.001 },
  },
  'SMALLINT': {
    sequential: { start: 1 },
    uniform: { min: 1, max: 500 },
    normal: { mean: 250, stddev: 80 },
  },
  'DOUBLE PRECISION': {
    uniform: { min: 0, max: 100 },
    normal: { mean: 50, stddev: 15 },
    lognormal: { mu: 3.5, sigma: 1.0, cap: 50000 },
    exponential: { lambda: 0.015, offset: 5 },
    bimodal: { mean1: 100, stddev1: 10, mean2: 200, stddev2: 15, weight: 0.6 },
  },
  'TEXT': {
    categorical: { values: ['Alpha', 'Beta', 'Gamma', 'Delta', 'Epsilon'], zipf: false, zipfExponent: 1.5 },
    uuid: {},
    words: { wordCount: 3 },
  },
  'BOOLEAN': {
    bernoulli: { trueProbability: 50 },
  },
  'DATE': {
    uniform_range: { start: '2020-01-01', end: '2025-12-31', daySpan: 2190 },
  },
  'TIMESTAMP': {
    uniform_range: { start: '2020-01-01 00:00:00', secondSpan: 189216000 },
  },
};

function defaultColumn(type: string, dist: string, name: string): GenerateColumnSpec {
  const params = DEFAULT_PARAMS[type]?.[dist] ?? {};
  return {
    name,
    type,
    nullable: false,
    nullPercent: 0,
    distribution: dist,
    params: { ...params },
  };
}

function defaultTable(index: number): GenerateTableSpec {
  return {
    name: `table_${index + 1}`,
    rowCount: 100000,
    columns: [
      defaultColumn('INT', 'sequential', 'id'),
      defaultColumn('TEXT', 'categorical', 'category'),
      defaultColumn('DOUBLE PRECISION', 'normal', 'value'),
      defaultColumn('BOOLEAN', 'bernoulli', 'flag'),
      defaultColumn('TIMESTAMP', 'uniform_range', 'created_at'),
    ],
  };
}

const STORAGE_KEY = 'hyper-explorer:generate-spec';

function loadSavedSpec(): { dbPath: string; tables: GenerateTableSpec[] } | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw);
  } catch {}
  return null;
}

function saveSpec(dbPath: string, tables: GenerateTableSpec[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ dbPath, tables }));
  } catch {}
}

export function GenerateDatabase({ onGenerated }: Props) {
  const [meta, setMeta] = useState<GenerateMetaResult | null>(null);
  const [dbPath, setDbPath] = useState(() => loadSavedSpec()?.dbPath ?? '/tmp/generated.hyper');
  const [tables, setTables] = useState<GenerateTableSpec[]>(() => loadSavedSpec()?.tables ?? [defaultTable(0)]);
  const [generating, setGenerating] = useState(false);
  const [result, setResult] = useState<GenerateResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Persist spec to localStorage on every change
  useEffect(() => {
    saveSpec(dbPath, tables);
  }, [dbPath, tables]);

  useEffect(() => {
    getGenerateMeta().then(setMeta).catch((e) => setError(e.message));
  }, []);

  const updateTable = useCallback((idx: number, updates: Partial<GenerateTableSpec>) => {
    setTables((prev) => prev.map((t, i) => (i === idx ? { ...t, ...updates } : t)));
  }, []);

  const updateColumn = useCallback((tIdx: number, cIdx: number, updates: Partial<GenerateColumnSpec>) => {
    setTables((prev) =>
      prev.map((t, ti) =>
        ti === tIdx
          ? { ...t, columns: t.columns.map((c, ci) => (ci === cIdx ? { ...c, ...updates } : c)) }
          : t,
      ),
    );
  }, []);

  const addTable = useCallback(() => {
    setTables((prev) => [...prev, defaultTable(prev.length)]);
  }, []);

  const removeTable = useCallback((idx: number) => {
    setTables((prev) => prev.filter((_, i) => i !== idx));
  }, []);

  const addColumn = useCallback((tIdx: number) => {
    setTables((prev) =>
      prev.map((t, i) =>
        i === tIdx
          ? { ...t, columns: [...t.columns, defaultColumn('DOUBLE PRECISION', 'uniform', `col_${t.columns.length + 1}`)] }
          : t,
      ),
    );
  }, []);

  const removeColumn = useCallback((tIdx: number, cIdx: number) => {
    setTables((prev) =>
      prev.map((t, i) =>
        i === tIdx ? { ...t, columns: t.columns.filter((_, ci) => ci !== cIdx) } : t,
      ),
    );
  }, []);

  const handleGenerate = useCallback(async () => {
    setGenerating(true);
    setError(null);
    setResult(null);
    try {
      const res = await generateDatabase({ dbPath, tables });
      setResult(res);
    } catch (err: any) {
      setError(err.message);
    } finally {
      setGenerating(false);
    }
  }, [dbPath, tables]);

  if (!meta) {
    return <div className="p-6 text-gray-400">Loading generator config...</div>;
  }

  const distOptions = meta.distributionsByType;

  return (
    <div className="p-6 max-w-5xl mx-auto space-y-6 overflow-auto h-full">
      <div>
        <h2 className="text-xl font-bold text-gray-100 mb-1">Generate Database</h2>
        <p className="text-sm text-gray-500">Configure tables, columns, and data distributions to create a new .hyper file.</p>
      </div>

      {/* DB path */}
      <div>
        <label className="block text-xs text-gray-400 mb-1">Database File Path</label>
        <input
          type="text"
          value={dbPath}
          onChange={(e) => setDbPath(e.target.value)}
          className="w-full max-w-lg bg-gray-800 border border-gray-700 rounded px-3 py-1.5 text-sm text-gray-200 focus:outline-none focus:border-blue-500"
          placeholder="/path/to/output.hyper"
        />
      </div>

      {/* Tables */}
      {tables.map((table, tIdx) => (
        <div key={tIdx} className="bg-gray-900 border border-gray-800 rounded-lg overflow-hidden">
          {/* Table header */}
          <div className="px-4 py-3 bg-gray-800/50 flex items-center gap-3 border-b border-gray-800">
            <input
              type="text"
              value={table.name}
              onChange={(e) => updateTable(tIdx, { name: e.target.value })}
              className="bg-transparent border border-gray-700 rounded px-2 py-1 text-sm text-gray-200 font-semibold focus:outline-none focus:border-blue-500 w-48"
            />
            <div className="flex items-center gap-1.5">
              <label className="text-xs text-gray-500">Rows:</label>
              <input
                type="number"
                value={table.rowCount}
                onChange={(e) => updateTable(tIdx, { rowCount: Math.max(1, parseInt(e.target.value) || 1) })}
                className="bg-gray-800 border border-gray-700 rounded px-2 py-1 text-sm text-gray-200 w-32 focus:outline-none focus:border-blue-500"
                min={1}
              />
            </div>
            <div className="ml-auto flex gap-2">
              <button
                onClick={() => addColumn(tIdx)}
                className="px-2.5 py-1 text-xs bg-blue-600/30 text-blue-400 rounded hover:bg-blue-600/50 transition-colors"
              >
                + Column
              </button>
              {tables.length > 1 && (
                <button
                  onClick={() => removeTable(tIdx)}
                  className="px-2.5 py-1 text-xs bg-red-600/20 text-red-400 rounded hover:bg-red-600/40 transition-colors"
                >
                  Remove Table
                </button>
              )}
            </div>
          </div>

          {/* Column list */}
          <div className="divide-y divide-gray-800/50">
            {table.columns.map((col, cIdx) => (
              <ColumnRow
                key={cIdx}
                col={col}
                distOptions={distOptions}
                onUpdate={(updates) => updateColumn(tIdx, cIdx, updates)}
                onRemove={table.columns.length > 1 ? () => removeColumn(tIdx, cIdx) : undefined}
              />
            ))}
          </div>
        </div>
      ))}

      <button
        onClick={addTable}
        className="px-4 py-2 bg-gray-800 border border-gray-700 text-gray-300 rounded hover:bg-gray-700 text-sm transition-colors"
      >
        + Add Table
      </button>

      {/* Generate button */}
      <div className="flex items-center gap-4 pt-2">
        <button
          onClick={handleGenerate}
          disabled={generating || !dbPath.trim() || tables.length === 0}
          className="px-6 py-2.5 bg-blue-600 hover:bg-blue-500 disabled:bg-gray-700 disabled:text-gray-500 text-white font-medium rounded transition-colors"
        >
          {generating ? 'Generating...' : 'Generate Database'}
        </button>
        {generating && (
          <span className="text-sm text-gray-400 flex items-center gap-2">
            <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
            Creating tables and generating data...
          </span>
        )}
      </div>

      {/* Error */}
      {error && (
        <div className="px-4 py-3 bg-red-900/30 border border-red-800/50 rounded text-red-300 text-sm">
          {error}
        </div>
      )}

      {/* Result */}
      {result && (
        <div className="bg-emerald-900/20 border border-emerald-800/40 rounded-lg p-4 space-y-3">
          <div className="text-emerald-300 font-medium text-sm">Database created successfully!</div>
          <div className="text-xs text-gray-400 font-mono">{result.dbPath}</div>
          <div className="space-y-1">
            {result.results.map((r, i) => (
              <div key={i} className="flex items-center gap-3 text-xs">
                <span className="text-gray-300 font-medium">{r.table}</span>
                <span className="text-gray-500">{r.rowCount.toLocaleString()} rows</span>
                <span className="text-gray-600">{(r.durationMs / 1000).toFixed(1)}s</span>
              </div>
            ))}
          </div>
          <button
            onClick={() => onGenerated(result.dbPath)}
            className="px-4 py-1.5 bg-blue-600 hover:bg-blue-500 text-white text-sm font-medium rounded transition-colors"
          >
            Open in Explorer
          </button>
        </div>
      )}
    </div>
  );
}

// =============================================================================
// Column Row Editor
// =============================================================================

function ColumnRow({
  col,
  distOptions,
  onUpdate,
  onRemove,
}: {
  col: GenerateColumnSpec;
  distOptions: Record<string, string[]>;
  onUpdate: (updates: Partial<GenerateColumnSpec>) => void;
  onRemove?: () => void;
}) {
  const dists = distOptions[col.type] || [];

  const handleTypeChange = (newType: string) => {
    const newDists = distOptions[newType] || [];
    const newDist = newDists[0] || 'uniform';
    const newParams = DEFAULT_PARAMS[newType]?.[newDist] ?? {};
    onUpdate({ type: newType, distribution: newDist, params: { ...newParams } });
  };

  const handleDistChange = (newDist: string) => {
    const newParams = DEFAULT_PARAMS[col.type]?.[newDist] ?? {};
    onUpdate({ distribution: newDist, params: { ...newParams } });
  };

  const handleParamChange = (key: string, value: any) => {
    onUpdate({ params: { ...col.params, [key]: value } });
  };

  return (
    <div className="px-4 py-2.5 flex flex-wrap items-center gap-2">
      {/* Column name */}
      <input
        type="text"
        value={col.name}
        onChange={(e) => onUpdate({ name: e.target.value })}
        className="bg-gray-800 border border-gray-700 rounded px-2 py-1 text-xs text-gray-200 w-28 focus:outline-none focus:border-blue-500"
        placeholder="column_name"
      />

      {/* Type selector */}
      <select
        value={col.type}
        onChange={(e) => handleTypeChange(e.target.value)}
        className="bg-gray-800 border border-gray-700 rounded px-2 py-1 text-xs text-gray-200 focus:outline-none focus:border-blue-500"
      >
        {Object.keys(distOptions).map((t) => (
          <option key={t} value={t}>{t}</option>
        ))}
      </select>

      {/* Distribution selector */}
      <select
        value={col.distribution}
        onChange={(e) => handleDistChange(e.target.value)}
        className="bg-gray-800 border border-gray-700 rounded px-2 py-1 text-xs text-gray-200 focus:outline-none focus:border-blue-500"
      >
        {dists.map((d) => (
          <option key={d} value={d}>{d}</option>
        ))}
      </select>

      {/* Nullable */}
      <label className="flex items-center gap-1 text-xs text-gray-400">
        <input
          type="checkbox"
          checked={col.nullable}
          onChange={(e) => onUpdate({ nullable: e.target.checked, nullPercent: e.target.checked ? (col.nullPercent || 10) : 0 })}
          className="rounded bg-gray-800 border-gray-600"
        />
        nullable
      </label>
      {col.nullable && (
        <div className="flex items-center gap-1">
          <input
            type="number"
            value={col.nullPercent}
            onChange={(e) => onUpdate({ nullPercent: Math.min(100, Math.max(0, parseInt(e.target.value) || 0)) })}
            className="bg-gray-800 border border-gray-700 rounded px-1.5 py-1 text-xs text-gray-200 w-14 focus:outline-none focus:border-blue-500"
            min={0}
            max={100}
          />
          <span className="text-[10px] text-gray-500">% null</span>
        </div>
      )}

      {/* Distribution params */}
      <ParamEditor col={col} onChange={handleParamChange} />

      {/* Remove button */}
      {onRemove && (
        <button
          onClick={onRemove}
          className="ml-auto text-gray-600 hover:text-red-400 transition-colors"
          title="Remove column"
        >
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      )}
    </div>
  );
}

// =============================================================================
// Distribution Parameter Editors
// =============================================================================

function ParamEditor({
  col,
  onChange,
}: {
  col: GenerateColumnSpec;
  onChange: (key: string, value: any) => void;
}) {
  const p = col.params;
  const numInput = (key: string, label: string, step?: number) => (
    <label key={key} className="flex items-center gap-1 text-[10px] text-gray-500">
      {label}:
      <input
        type="number"
        value={p[key] ?? ''}
        onChange={(e) => onChange(key, parseFloat(e.target.value) || 0)}
        step={step}
        className="bg-gray-800 border border-gray-700 rounded px-1.5 py-0.5 text-xs text-gray-300 w-20 focus:outline-none focus:border-blue-500"
      />
    </label>
  );
  const textInput = (key: string, label: string) => (
    <label key={key} className="flex items-center gap-1 text-[10px] text-gray-500">
      {label}:
      <input
        type="text"
        value={p[key] ?? ''}
        onChange={(e) => onChange(key, e.target.value)}
        className="bg-gray-800 border border-gray-700 rounded px-1.5 py-0.5 text-xs text-gray-300 w-28 focus:outline-none focus:border-blue-500"
      />
    </label>
  );

  switch (col.distribution) {
    case 'sequential':
      return <>{numInput('start', 'start')}</>;
    case 'uniform':
      return <>{numInput('min', 'min')}{numInput('max', 'max')}</>;
    case 'normal':
      return <>{numInput('mean', 'mean')}{numInput('stddev', 'stddev')}</>;
    case 'lognormal':
      return <>{numInput('mu', 'mu', 0.1)}{numInput('sigma', 'sigma', 0.1)}{numInput('cap', 'cap')}</>;
    case 'exponential':
      return <>{numInput('lambda', 'λ', 0.001)}{numInput('offset', 'offset')}</>;
    case 'bimodal':
      return (
        <>
          {numInput('mean1', 'μ1')}{numInput('stddev1', 'σ1')}
          {numInput('mean2', 'μ2')}{numInput('stddev2', 'σ2')}
          {numInput('weight', 'w', 0.1)}
        </>
      );
    case 'categorical': {
      const vals = (p.values || []).join(', ');
      return (
        <>
          <label className="flex items-center gap-1 text-[10px] text-gray-500">
            values:
            <input
              type="text"
              value={vals}
              onChange={(e) => onChange('values', e.target.value.split(',').map((s: string) => s.trim()).filter(Boolean))}
              className="bg-gray-800 border border-gray-700 rounded px-1.5 py-0.5 text-xs text-gray-300 w-48 focus:outline-none focus:border-blue-500"
              placeholder="A, B, C, D"
            />
          </label>
          <label className="flex items-center gap-1 text-[10px] text-gray-500">
            <input
              type="checkbox"
              checked={p.zipf ?? false}
              onChange={(e) => onChange('zipf', e.target.checked)}
              className="rounded bg-gray-800 border-gray-600"
            />
            Zipf
          </label>
          {p.zipf && numInput('zipfExponent', 'exp', 0.1)}
        </>
      );
    }
    case 'words':
      return <>{numInput('wordCount', 'words')}</>;
    case 'bernoulli':
      return <>{numInput('trueProbability', 'true%')}</>;
    case 'uniform_range':
      if (col.type === 'DATE') {
        return <>{textInput('start', 'from')}{textInput('end', 'to')}{numInput('daySpan', 'days')}</>;
      }
      return <>{textInput('start', 'from')}{numInput('secondSpan', 'seconds')}</>;
    case 'uuid':
      return null;
    default:
      return null;
  }
}
