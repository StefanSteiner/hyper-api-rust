import { useState, useCallback, useMemo, useRef, type FormEvent } from 'react';
import { DataGrid } from './DataGrid';
import { QueryStatsTooltip } from './QueryStatsTooltip';
import { executeQuery, type QueryResult, type ColumnInfo, type SchemaNode, type QueryStats } from '../api';

interface Props {
  db: string;
  schemas?: SchemaNode[];
  onQueryExecuted?: (info: { sql: string; rowCount: number | null; durationMs: number; type: 'query' | 'command'; queryStats?: QueryStats }) => void;
}

function quoteIdent(name: string): string {
  return `"${name.replace(/"/g, '""')}"`;
}

function generateExamples(schemas: SchemaNode[]): { label: string; sql: string }[] {
  const examples: { label: string; sql: string }[] = [];
  for (const s of schemas) {
    for (const t of s.tables) {
      const fqn = `${quoteIdent(s.schema)}.${quoteIdent(t.name)}`;
      const cols = t.columns;

      examples.push({ label: `Preview ${t.name}`, sql: `SELECT * FROM ${fqn} LIMIT 100` });
      examples.push({ label: `Count ${t.name}`, sql: `SELECT COUNT(*) AS total FROM ${fqn}` });

      if (cols.length > 0) {
        const numCol = cols.find((c) => /int|float|double|numeric|decimal|bigint|smallint|real/i.test(c.typeName));
        if (numCol) {
          examples.push({
            label: `Stats on ${t.name}.${numCol.name}`,
            sql: `SELECT\n  MIN(${quoteIdent(numCol.name)}) AS min_val,\n  MAX(${quoteIdent(numCol.name)}) AS max_val,\n  AVG(${quoteIdent(numCol.name)}) AS avg_val,\n  COUNT(*) AS total\nFROM ${fqn}`,
          });
        }

        const textCol = cols.find((c) => /varchar|text|char/i.test(c.typeName));
        if (textCol) {
          examples.push({
            label: `Top ${textCol.name} in ${t.name}`,
            sql: `SELECT ${quoteIdent(textCol.name)}, COUNT(*) AS cnt\nFROM ${fqn}\nGROUP BY ${quoteIdent(textCol.name)}\nORDER BY cnt DESC\nLIMIT 10`,
          });
        }
      }
    }
  }
  return examples;
}

export function SqlEditor({ db, schemas, onQueryExecuted }: Props) {
  const [sql, setSql] = useState('');
  const [result, setResult] = useState<QueryResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const examples = useMemo(() => generateExamples(schemas ?? []), [schemas]);

  const handleRun = useCallback(
    async (e?: FormEvent) => {
      e?.preventDefault();
      const trimmed = sql.trim();
      if (!trimmed) return;

      setLoading(true);
      setError(null);
      setResult(null);
      try {
        const res = await executeQuery(db, trimmed);
        setResult(res);
        onQueryExecuted?.({
          sql: trimmed,
          rowCount: res.type === 'query' ? res.rowCount : (res.rowsAffected ?? null),
          durationMs: res.durationMs,
          type: res.type,
          queryStats: res.queryStats,
        });
      } catch (err: any) {
        setError(err.message);
      } finally {
        setLoading(false);
      }
    },
    [db, sql],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
        e.preventDefault();
        handleRun();
      }
    },
    [handleRun],
  );

  return (
    <div className="flex flex-col h-full">
      {/* Editor */}
      <div className="p-4 border-b border-gray-800 shrink-0">
        <textarea
          ref={textareaRef}
          value={sql}
          onChange={(e) => setSql(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="SELECT * FROM ..."
          rows={5}
          className="w-full bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm text-gray-200 font-mono placeholder-gray-500 focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500 resize-y"
          disabled={loading}
          spellCheck={false}
        />
        <div className="flex items-center gap-3 mt-2">
          <button
            onClick={() => handleRun()}
            disabled={loading || !sql.trim()}
            className="px-4 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:bg-gray-700 disabled:text-gray-500 text-white text-sm font-medium rounded transition-colors"
          >
            {loading ? 'Running...' : 'Run'}
          </button>
          <span className="text-xs text-gray-500">
            {navigator.platform.includes('Mac') ? '⌘' : 'Ctrl'}+Enter to run
          </span>
          {result && (
            <span className="text-xs text-gray-500 ml-auto">
              <QueryStatsTooltip stats={result.queryStats ?? null}>
                <span>{result.durationMs}ms</span>
              </QueryStatsTooltip>
              {result.type === 'query' && ` · ${result.rowCount} rows`}
              {result.type === 'command' && ` · ${result.rowsAffected} rows affected`}
            </span>
          )}
        </div>
      </div>

      {/* Error */}
      {error && (
        <div className="px-4 py-3 bg-red-900/30 border-b border-red-800/50 text-red-300 text-sm font-mono">
          {error}
        </div>
      )}

      {/* Results */}
      <div className="flex-1 overflow-auto">
        {result?.type === 'query' && (
          <DataGrid columns={result.columns} rows={result.rows} />
        )}
        {result?.type === 'command' && (
          <div className="p-4 text-green-400 text-sm">
            Command executed successfully. {result.rowsAffected} row(s) affected.
          </div>
        )}
        {!result && !error && examples.length > 0 && (
          <div className="px-4 py-4">
            <p className="text-xs text-gray-500 mb-3">Example queries — click to load</p>
            <div className="flex flex-wrap gap-2">
              {examples.map((ex, i) => (
                <button
                  key={i}
                  onClick={() => setSql(ex.sql)}
                  className="px-3 py-1.5 bg-gray-800 hover:bg-gray-700 border border-gray-700 rounded text-xs text-gray-300 transition-colors text-left"
                  title={ex.sql}
                >
                  {ex.label}
                </button>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
