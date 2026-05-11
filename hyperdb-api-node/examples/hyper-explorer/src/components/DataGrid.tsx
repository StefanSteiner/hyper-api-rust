import { useState, useRef, useMemo, useCallback, type UIEvent, type PointerEvent as ReactPointerEvent } from 'react';
import type { ColumnInfo } from '../api';

type SortDir = 'asc' | 'desc';

interface SortState {
  column: string;
  dir: SortDir;
}

function compareValues(a: string | null | undefined, b: string | null | undefined): number {
  if (a == null && b == null) return 0;
  if (a == null) return 1;
  if (b == null) return -1;
  const numA = Number(a);
  const numB = Number(b);
  if (!isNaN(numA) && !isNaN(numB)) return numA - numB;
  return String(a).localeCompare(String(b));
}

interface Props {
  columns: ColumnInfo[];
  rows: Record<string, string | null>[];
  rowOffset?: number;
  totalRowCount?: number;
  hasMore?: boolean;
  loadingMore?: boolean;
  onLoadMore?: () => void;
  sort?: SortState | null;
  onSort?: (sort: SortState | null) => void;
}

// Default column width in pixels
const DEFAULT_COL_WIDTH = 150;
const MIN_COL_WIDTH = 50;
const ROW_NUM_WIDTH = 52;

export function DataGrid({ columns, rows, rowOffset = 0, totalRowCount, hasMore, loadingMore, onLoadMore, sort: controlledSort, onSort }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [localSort, setLocalSort] = useState<SortState | null>(null);
  const [colWidths, setColWidths] = useState<Record<string, number>>({});

  // Only reset column widths when the set of column names actually changes
  // (not on re-renders from sorting or data refresh with the same columns)
  const colKey = columns.map((c) => c.name).join('\0');
  const prevColKey = useRef<string | null>(null);
  if (prevColKey.current !== null && prevColKey.current !== colKey) {
    // Column set changed (e.g., switched tables) — reset widths
    setColWidths({});
  }
  prevColKey.current = colKey;

  // Resize drag state (refs to avoid re-renders during drag)
  const dragState = useRef<{ col: string; startX: number; startW: number } | null>(null);

  const handleResizeStart = useCallback((col: string, e: ReactPointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const startW = colWidths[col] ?? DEFAULT_COL_WIDTH;
    dragState.current = { col, startX: e.clientX, startW };
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }, [colWidths]);

  const handleResizeMove = useCallback((e: ReactPointerEvent) => {
    if (!dragState.current) return;
    const { col, startX, startW } = dragState.current;
    const newW = Math.max(MIN_COL_WIDTH, startW + (e.clientX - startX));
    setColWidths((prev) => ({ ...prev, [col]: newW }));
  }, []);

  const handleResizeEnd = useCallback(() => {
    dragState.current = null;
  }, []);

  // Use controlled sort if provided, otherwise local sort
  const sort = onSort ? (controlledSort ?? null) : localSort;

  const handleSort = useCallback((colName: string) => {
    const next = (prev: SortState | null): SortState | null => {
      if (prev && prev.column === colName) {
        return prev.dir === 'asc' ? { column: colName, dir: 'desc' } : null;
      }
      return { column: colName, dir: 'asc' };
    };
    if (onSort) {
      onSort(next(controlledSort ?? null));
    } else {
      setLocalSort((prev) => next(prev));
    }
  }, [onSort, controlledSort]);

  const sortedRows = useMemo(() => {
    // When server-side sorting is active (onSort provided), rows are already sorted
    if (onSort) return rows;
    if (!localSort) return rows;
    const { column, dir } = localSort;
    const sorted = [...rows].sort((a, b) => compareValues(a[column], b[column]));
    if (dir === 'desc') sorted.reverse();
    return sorted;
  }, [rows, localSort, onSort]);

  const handleScroll = useCallback(
    (e: UIEvent<HTMLDivElement>) => {
      if (!hasMore || loadingMore || !onLoadMore) return;
      const el = e.currentTarget;
      if (el.scrollTop + el.clientHeight > el.scrollHeight * 0.5) {
        onLoadMore();
      }
    },
    [hasMore, loadingMore, onLoadMore],
  );

  // Compute total table width for fixed layout
  const tableWidth = ROW_NUM_WIDTH + columns.reduce((sum, col) => sum + (colWidths[col.name] ?? DEFAULT_COL_WIDTH), 0);

  if (columns.length === 0) {
    return <div className="p-4 text-gray-500">No columns</div>;
  }

  return (
    <div ref={scrollRef} className="overflow-auto h-full" onScroll={handleScroll}>
      <table className="text-sm border-collapse" style={{ width: tableWidth, tableLayout: 'fixed' }}>
        <colgroup>
          <col style={{ width: ROW_NUM_WIDTH }} />
          {columns.map((col) => (
            <col key={col.name} style={{ width: colWidths[col.name] ?? DEFAULT_COL_WIDTH }} />
          ))}
        </colgroup>
        <thead className="sticky top-0 z-10">
          <tr className="bg-gray-800">
            <th className="px-3 py-2 text-left text-xs font-semibold text-gray-400 border-b border-gray-700">
              #
            </th>
            {columns.map((col) => {
              const isActive = sort?.column === col.name;
              return (
                <th
                  key={col.name}
                  className="relative px-3 py-2 text-left text-xs font-semibold text-gray-300 border-b border-gray-700 whitespace-nowrap cursor-pointer select-none hover:bg-gray-700/50 transition-colors group"
                  onClick={() => handleSort(col.name)}
                >
                  <div className="flex items-center gap-1 overflow-hidden">
                    <span className="truncate">{col.name}</span>
                    <span className={`text-[10px] shrink-0 ${isActive ? 'text-blue-400' : 'text-gray-600'}`}>
                      {isActive ? (sort!.dir === 'asc' ? '▲' : '▼') : '⇅'}
                    </span>
                  </div>
                  <div className="font-normal text-gray-500 truncate">{col.typeName}</div>
                  {/* Resize handle */}
                  <div
                    className="absolute top-0 right-0 w-2 h-full cursor-col-resize flex items-center justify-center opacity-0 group-hover:opacity-100 hover:!opacity-100 z-20"
                    onPointerDown={(e) => handleResizeStart(col.name, e)}
                    onPointerMove={handleResizeMove}
                    onPointerUp={handleResizeEnd}
                    onClick={(e) => e.stopPropagation()}
                  >
                    <div className="w-[3px] h-4/5 rounded-full bg-gray-500 hover:bg-blue-400 transition-colors" />
                  </div>
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {sortedRows.map((row, i) => (
            <tr
              key={rowOffset + i}
              className="border-b border-gray-800/50 hover:bg-gray-800/30 transition-colors"
            >
              <td className="px-3 py-1.5 text-gray-600 text-xs">{rowOffset + i + 1}</td>
              {columns.map((col) => {
                const val = row[col.name];
                const isNull = val === null || val === undefined;
                return (
                  <td
                    key={col.name}
                    className={`px-3 py-1.5 whitespace-nowrap overflow-hidden text-ellipsis ${
                      isNull ? 'text-gray-600 italic' : 'text-gray-200'
                    }`}
                    title={isNull ? 'NULL' : String(val)}
                  >
                    {isNull ? 'NULL' : String(val)}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
      <div className="px-3 py-2 text-xs text-gray-500 border-t border-gray-800 flex items-center gap-2" style={{ minWidth: tableWidth }}>
        {loadingMore && (
          <svg className="animate-spin h-3.5 w-3.5 text-gray-400" viewBox="0 0 24 24">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
          </svg>
        )}
        <span>
          {rows.length}{totalRowCount != null ? ` of ${totalRowCount.toLocaleString()}` : ''} row{rows.length !== 1 ? 's' : ''} shown
          {hasMore === false && totalRowCount != null && ' (all loaded)'}
        </span>
      </div>
    </div>
  );
}
