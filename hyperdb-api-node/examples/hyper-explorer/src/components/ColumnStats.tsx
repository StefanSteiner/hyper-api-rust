import type { ColumnStat } from '../api';

interface Props {
  stats: ColumnStat[];
}

export function ColumnStats({ stats }: Props) {
  if (stats.length === 0) {
    return <div className="p-4 text-gray-500">No column statistics available</div>;
  }

  return (
    <div className="p-4 grid gap-4 grid-cols-1 md:grid-cols-2 xl:grid-cols-3">
      {stats.map((col) => (
        <div
          key={col.name}
          className="bg-gray-900 border border-gray-800 rounded-lg p-4"
        >
          {/* Column header */}
          <div className="flex items-center justify-between mb-3 pb-2 border-b border-gray-800">
            <div>
              <h3 className="text-sm font-semibold text-gray-200">{col.name}</h3>
              <span className="text-xs text-gray-500">{col.typeName}</span>
            </div>
            <TypeBadge typeName={col.typeName} />
          </div>

          {/* Universal stats */}
          <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 text-xs mb-3">
            <StatRow label="Rows" value={col.rowCount.toLocaleString()} />
            <StatRow label="Nulls" value={`${col.nullCount.toLocaleString()} (${col.nullPercent}%)`} />
            <StatRow label="Distinct" value={col.distinctCount.toLocaleString()} />
          </div>

          {/* Null bar */}
          <div className="mb-3">
            <div className="h-1.5 bg-gray-800 rounded-full overflow-hidden">
              <div
                className="h-full bg-amber-500/60 rounded-full transition-all"
                style={{ width: `${Math.min(col.nullPercent, 100)}%` }}
              />
            </div>
            <div className="flex justify-between text-[10px] text-gray-600 mt-0.5">
              <span>0% null</span>
              <span>100%</span>
            </div>
          </div>

          {/* Type-specific stats */}
          <TypeSpecificStats col={col} />
        </div>
      ))}
    </div>
  );
}

function StatRow({ label, value }: { label: string; value: string }) {
  return (
    <>
      <span className="text-gray-500">{label}</span>
      <span className="text-gray-300 text-right">{value}</span>
    </>
  );
}

function TypeBadge({ typeName }: { typeName: string }) {
  const t = typeName.toUpperCase();
  let color = 'bg-gray-700 text-gray-400';
  if (t.includes('INT') || t.includes('DOUBLE') || t.includes('FLOAT') || t.includes('NUMERIC')) {
    color = 'bg-blue-900/50 text-blue-400';
  } else if (t.includes('TEXT') || t.includes('VARCHAR') || t.includes('CHAR')) {
    color = 'bg-green-900/50 text-green-400';
  } else if (t === 'BOOL' || t === 'BOOLEAN') {
    color = 'bg-purple-900/50 text-purple-400';
  } else if (t.includes('DATE') || t.includes('TIMESTAMP')) {
    color = 'bg-orange-900/50 text-orange-400';
  }
  return (
    <span className={`px-2 py-0.5 rounded text-[10px] font-medium ${color}`}>
      {t.includes('INT') || t.includes('DOUBLE') || t.includes('FLOAT') || t.includes('NUMERIC')
        ? 'NUM'
        : t.includes('TEXT') || t.includes('VARCHAR')
          ? 'TXT'
          : t === 'BOOL' || t === 'BOOLEAN'
            ? 'BOOL'
            : t.includes('DATE') || t.includes('TIMESTAMP')
              ? 'DATE'
              : 'OTHER'}
    </span>
  );
}

function TypeSpecificStats({ col }: { col: ColumnStat }) {
  const t = col.typeName.toUpperCase();

  // Numeric
  if (t.includes('INT') || t.includes('DOUBLE') || t.includes('FLOAT') || t.includes('NUMERIC')) {
    return (
      <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 text-xs">
        <StatRow label="Min" value={col.min != null ? String(col.min) : '—'} />
        <StatRow label="Max" value={col.max != null ? String(col.max) : '—'} />
        <StatRow
          label="Mean"
          value={col.mean != null ? Number(col.mean).toFixed(2) : '—'}
        />
        <StatRow
          label="Std Dev"
          value={col.stddev != null ? Number(col.stddev).toFixed(2) : '—'}
        />
        <StatRow
          label="CV"
          value={col.cv != null ? `${col.cv}%` : '—'}
        />
      </div>
    );
  }

  // Text
  if (t.includes('TEXT') || t.includes('VARCHAR') || t.includes('CHAR')) {
    return (
      <div>
        <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 text-xs mb-2">
          <StatRow label="Min Len" value={col.minLength != null ? String(col.minLength) : '—'} />
          <StatRow label="Max Len" value={col.maxLength != null ? String(col.maxLength) : '—'} />
          <StatRow
            label="Avg Len"
            value={col.avgLength != null ? Number(col.avgLength).toFixed(1) : '—'}
          />
        </div>
        {col.topValues && col.topValues.length > 0 && (
          <div className="mt-2">
            <div className="text-[10px] text-gray-500 uppercase tracking-wide mb-1">Top Values</div>
            {col.topValues.map((tv, i) => (
              <div key={i} className="flex items-center justify-between text-xs py-0.5">
                <span className="text-gray-300 truncate mr-2" title={tv.value}>
                  {tv.value || '(empty)'}
                </span>
                <span className="text-gray-500 shrink-0">{tv.count}</span>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  }

  // Bool
  if (t === 'BOOL' || t === 'BOOLEAN') {
    return (
      <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 text-xs">
        <StatRow label="True" value={col.trueCount != null ? String(col.trueCount) : '—'} />
        <StatRow label="False" value={col.falseCount != null ? String(col.falseCount) : '—'} />
        <StatRow
          label="True %"
          value={col.truePercent != null ? `${col.truePercent}%` : '—'}
        />
      </div>
    );
  }

  // Date
  if (t.includes('DATE') || t.includes('TIMESTAMP')) {
    return (
      <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 text-xs">
        <StatRow label="Min" value={col.min != null ? String(col.min) : '—'} />
        <StatRow label="Max" value={col.max != null ? String(col.max) : '—'} />
      </div>
    );
  }

  return null;
}
