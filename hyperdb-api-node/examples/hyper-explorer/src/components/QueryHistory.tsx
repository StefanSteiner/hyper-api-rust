import { useState, useMemo, type ReactNode } from 'react';
import { format } from 'sql-formatter';
import { QueryStatsTooltip } from './QueryStatsTooltip';
import type { QueryStats } from '../api';

// ── SQL formatting via sql-formatter (PostgreSQL dialect) ───────────
function beautifySql(sql: string): string {
  try {
    return format(sql, { language: 'postgresql', tabWidth: 2, keywordCase: 'upper' });
  } catch {
    return sql;
  }
}

// ── Syntax highlight tokens ─────────────────────────────────────────
const SQL_KW_RE = /\b(SELECT|FROM|WHERE|GROUP|BY|ORDER|HAVING|LIMIT|OFFSET|JOIN|LEFT|RIGHT|INNER|CROSS|FULL|OUTER|ON|AND|OR|NOT|IN|IS|NULL|AS|CASE|WHEN|THEN|ELSE|END|BETWEEN|LIKE|EXISTS|DISTINCT|ALL|ANY|UNION|INTERSECT|EXCEPT|INSERT|INTO|UPDATE|SET|DELETE|CREATE|ALTER|DROP|TABLE|VALUES|WITH|COUNT|SUM|AVG|MIN|MAX|CAST|COALESCE|ASC|DESC|NULLS|FIRST|LAST|TRUE|FALSE|BOOLEAN|INT|INTEGER|BIGINT|SMALLINT|DOUBLE|FLOAT|TEXT|VARCHAR|DATE|TIMESTAMP|INTERVAL)\b/gi;
const SQL_STR_RE = /('(?:[^']|'')*')/g;
const SQL_NUM_RE = /\b(\d+(?:\.\d+)?)\b/g;

function highlightSql(formatted: string): ReactNode[] {
  return formatted.split('\n').map((line, li) => {
    const parts: ReactNode[] = [];
    let last = 0;
    const combined = new RegExp(`${SQL_STR_RE.source}|${SQL_KW_RE.source}|${SQL_NUM_RE.source}`, 'gi');
    let m: RegExpExecArray | null;
    while ((m = combined.exec(line)) !== null) {
      if (m.index > last) parts.push(line.slice(last, m.index));
      const text = m[0];
      if (m[1]) {
        parts.push(<span key={`${li}-${m.index}`} className="text-amber-300">{text}</span>);
      } else if (m[2]) {
        parts.push(<span key={`${li}-${m.index}`} className="text-blue-400 font-semibold">{text.toUpperCase()}</span>);
      } else {
        parts.push(<span key={`${li}-${m.index}`} className="text-emerald-400">{text}</span>);
      }
      last = m.index + text.length;
    }
    if (last < line.length) parts.push(line.slice(last));
    return (
      <div key={li} className="whitespace-pre">{parts.length ? parts : ' '}</div>
    );
  });
}

// ── Click-to-open SQL modal ─────────────────────────────────────────
function SqlModal({ sql, onClose }: { sql: string; onClose: () => void }) {
  const formatted = useMemo(() => beautifySql(sql), [sql]);

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center pt-[15vh]" onClick={onClose}>
      <div className="absolute inset-0 bg-black/60" />
      <div
        className="relative bg-gray-950 border border-gray-700 rounded-lg shadow-2xl w-full max-w-2xl max-h-[60vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-2.5 border-b border-gray-800">
          <span className="text-xs font-medium text-gray-400">Formatted SQL</span>
          <button
            onClick={onClose}
            className="text-gray-500 hover:text-gray-300 text-lg leading-none transition-colors"
          >
            ×
          </button>
        </div>
        <div className="flex-1 overflow-auto px-4 py-3 text-xs font-mono leading-relaxed text-gray-200">
          {highlightSql(formatted)}
        </div>
        <div className="flex justify-end gap-2 px-4 py-2 border-t border-gray-800">
          <button
            onClick={() => { navigator.clipboard.writeText(formatted); }}
            className="px-3 py-1 text-xs text-gray-400 hover:text-gray-200 bg-gray-800 hover:bg-gray-700 rounded transition-colors"
          >
            Copy formatted
          </button>
          <button
            onClick={onClose}
            className="px-3 py-1 text-xs text-gray-400 hover:text-gray-200 bg-gray-800 hover:bg-gray-700 rounded transition-colors"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

export interface QueryHistoryEntry {
  id: number;
  sql: string;
  rowCount: number | null;
  durationMs: number;
  type: 'query' | 'command';
  source: string;
  connectionId?: number;
  queryStats?: QueryStats;
  timestamp: Date;
}

interface Props {
  entries: QueryHistoryEntry[];
  onRerun?: (sql: string) => void;
}

export function QueryHistory({ entries, onRerun }: Props) {
  const [modalSql, setModalSql] = useState<string | null>(null);

  if (entries.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center text-gray-600">
        <div className="text-center">
          <p className="text-xl mb-2">No queries yet</p>
          <p className="text-sm">Run queries from the SQL Editor tab and they'll appear here</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-auto">
      {modalSql && <SqlModal sql={modalSql} onClose={() => setModalSql(null)} />}
      <table className="w-full text-sm">
        <thead className="sticky top-0 bg-gray-900 border-b border-gray-700">
          <tr>
            <th className="text-left px-4 py-2.5 text-gray-400 font-medium w-10">#</th>
            <th className="text-left px-4 py-2.5 text-gray-400 font-medium w-24">Source</th>
            <th className="text-right px-4 py-2.5 text-gray-400 font-medium w-16">Conn</th>
            <th className="text-left px-4 py-2.5 text-gray-400 font-medium">SQL</th>
            <th className="text-right px-4 py-2.5 text-gray-400 font-medium w-24">Rows</th>
            <th className="text-right px-4 py-2.5 text-gray-400 font-medium w-24">Duration</th>
            <th className="text-right px-4 py-2.5 text-gray-400 font-medium w-44">Time</th>
            {onRerun && <th className="px-4 py-2.5 w-16" />}
          </tr>
        </thead>
        <tbody>
          {entries.map((entry, i) => (
            <tr
              key={entry.id}
              className="border-b border-gray-800/50 hover:bg-gray-800/40 transition-colors"
            >
              <td className="px-4 py-2 text-gray-500 tabular-nums">{entries.length - i}</td>
              <td className="px-4 py-2">
                <span className={`inline-block px-1.5 py-0.5 rounded text-[10px] font-medium ${
                  entry.source === 'user'
                    ? 'bg-blue-900/50 text-blue-300'
                    : 'bg-gray-800 text-gray-500'
                }`}>
                  {entry.source}
                </span>
              </td>
              <td className="px-4 py-2 text-right tabular-nums text-gray-500 text-xs">
                {entry.connectionId != null ? `#${entry.connectionId}` : '—'}
              </td>
              <td
                className="px-4 py-2 font-mono text-gray-300 max-w-md cursor-pointer hover:text-blue-300 transition-colors"
                onClick={() => setModalSql(entry.sql)}
                title="Click to view formatted SQL"
              >
                <span className="block truncate">{entry.sql}</span>
              </td>
              <td className="px-4 py-2 text-right tabular-nums text-gray-300">
                {entry.type === 'query'
                  ? (entry.rowCount ?? 0).toLocaleString()
                  : <span className="text-gray-500">{entry.rowCount != null ? `${entry.rowCount} affected` : '—'}</span>
                }
              </td>
              <td className="px-4 py-2 text-right tabular-nums text-gray-300">
                <QueryStatsTooltip stats={entry.queryStats ?? null}>
                  <span>{entry.durationMs.toLocaleString()} ms</span>
                </QueryStatsTooltip>
              </td>
              <td className="px-4 py-2 text-right text-gray-500 text-xs tabular-nums">
                {formatTime(entry.timestamp)}
              </td>
              {onRerun && (
                <td className="px-4 py-2 text-right">
                  <button
                    onClick={() => onRerun(entry.sql)}
                    className="text-blue-400 hover:text-blue-300 text-xs transition-colors"
                    title="Copy to SQL Editor"
                  >
                    rerun
                  </button>
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function formatTime(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}
