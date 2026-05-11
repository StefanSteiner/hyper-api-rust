import { useState, useCallback, useEffect, useRef, type DragEvent } from 'react';
import { FileOpener } from './components/FileOpener';
import { SchemaTree } from './components/SchemaTree';
import { DataGrid } from './components/DataGrid';
import { ColumnStats } from './components/ColumnStats';
import { ColumnDetailView } from './components/ColumnDetailView';
import { SqlEditor } from './components/SqlEditor';
import { GenerateDatabase } from './components/GenerateDatabase';
import { QueryHistory, type QueryHistoryEntry } from './components/QueryHistory';
import {
  openDatabase,
  getPreview,
  getStats,
  setOnInternalQueries,
  type OpenResult,
  type SchemaNode,
  type StatsResult,
  type ColumnInfo,
  type TrackedQuery,
  type QueryStats,
} from './api';

type Tab = 'preview' | 'stats' | 'column' | 'sql';

interface SelectedTable {
  schema: string;
  table: string;
}

export default function App() {
  const [dbPath, setDbPath] = useState<string | null>(null);
  const [schemas, setSchemas] = useState<SchemaNode[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showGenerator, setShowGenerator] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  const [showEditor, setShowEditor] = useState(false);
  const [queryHistory, setQueryHistory] = useState<QueryHistoryEntry[]>([]);
  const nextId = useRef(1);

  const addHistoryEntries = useCallback(
    (entries: { sql: string; rowCount: number | null; durationMs: number; type: 'query' | 'command'; source: string; connectionId?: number; queryStats?: QueryStats }[]) => {
      setQueryHistory((prev) => {
        const newEntries = entries.map((e) => ({
          id: nextId.current++,
          sql: e.sql,
          rowCount: e.rowCount,
          durationMs: e.durationMs,
          type: e.type,
          source: e.source,
          connectionId: e.connectionId,
          queryStats: e.queryStats,
          timestamp: new Date(),
        }));
        return [...newEntries, ...prev];
      });
    },
    [],
  );

  // Capture internal queries from API responses
  useEffect(() => {
    setOnInternalQueries((queries: TrackedQuery[]) => {
      addHistoryEntries(queries.map((q) => ({
        sql: q.sql,
        rowCount: q.rowCount,
        durationMs: q.durationMs,
        type: 'query' as const,
        source: q.source,
        connectionId: q.connectionId,
        queryStats: q.queryStats,
      })));
    });
    return () => setOnInternalQueries(null);
  }, [addHistoryEntries]);

  const handleQueryExecuted = useCallback(
    (info: { sql: string; rowCount: number | null; durationMs: number; type: 'query' | 'command'; queryStats?: QueryStats }) => {
      addHistoryEntries([{ ...info, source: 'user' }]);
    },
    [addHistoryEntries],
  );

  const [selected, setSelected] = useState<SelectedTable | null>(null);
  const [activeTab, setActiveTab] = useState<Tab>('preview');

  const [previewColumns, setPreviewColumns] = useState<ColumnInfo[]>([]);
  const [previewRows, setPreviewRows] = useState<Record<string, string | null>[]>([]);
  const [previewTotal, setPreviewTotal] = useState(0);
  const [previewHasMore, setPreviewHasMore] = useState(false);
  const [statsData, setStatsData] = useState<StatsResult | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewLoadingMore, setPreviewLoadingMore] = useState(false);
  const [statsLoading, setStatsLoading] = useState(false);
  const [previewSort, setPreviewSort] = useState<{ column: string; dir: 'asc' | 'desc' } | null>(null);

  const [selectedColumn, setSelectedColumn] = useState<{
    schema: string; table: string; column: string;
  } | null>(null);

  const handleOpen = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    setSelected(null);
    setSelectedColumn(null);
    setPreviewColumns([]);
    setPreviewRows([]);
    setPreviewTotal(0);
    setPreviewHasMore(false);
    setStatsData(null);
    try {
      const result: OpenResult = await openDatabase(path);
      setDbPath(result.database);
      setSchemas(result.schemas);
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }, []);

  const handleSelectTable = useCallback(
    async (schema: string, table: string) => {
      if (!dbPath) return;
      setSelected({ schema, table });
      setSelectedColumn(null);
      setActiveTab('preview');
      setPreviewColumns([]);
      setPreviewRows([]);
      setPreviewTotal(0);
      setPreviewHasMore(false);
      setStatsData(null);
      setPreviewSort(null);

      // Load preview (first page) — ask the server for the total row
      // count on initial load only. Paging / sorting reuse the cached total.
      setPreviewLoading(true);
      try {
        const result = await getPreview(dbPath, schema, table, 200, 0, undefined, undefined, true);
        setPreviewColumns(result.columns);
        setPreviewRows(result.rows);
        setPreviewTotal(result.totalRowCount ?? 0);
        setPreviewHasMore(
          result.totalRowCount != null
            ? result.rows.length < result.totalRowCount
            : result.rows.length === 200
        );
      } catch (err: any) {
        setError(err.message);
      } finally {
        setPreviewLoading(false);
      }

      // Load stats in background
      setStatsLoading(true);
      try {
        const result = await getStats(dbPath, schema, table);
        setStatsData(result);
      } catch (err: any) {
        console.error('Stats error:', err);
      } finally {
        setStatsLoading(false);
      }
    },
    [dbPath],
  );

  const handleLoadMorePreview = useCallback(async () => {
    if (!dbPath || !selected || previewLoadingMore) return;
    setPreviewLoadingMore(true);
    try {
      const result = await getPreview(
        dbPath, selected.schema, selected.table, 200, previewRows.length,
        previewSort?.column, previewSort?.dir,
      );
      setPreviewRows((prev) => {
        const newRows = [...prev, ...result.rows];
        // Server only returns totalRowCount on initial load; fall back
        // to the cached value and infer hasMore from the page fill.
        setPreviewHasMore(
          previewTotal > 0
            ? newRows.length < previewTotal
            : result.rows.length === 200
        );
        return newRows;
      });
    } catch (err: any) {
      setError(err.message);
    } finally {
      setPreviewLoadingMore(false);
    }
  }, [dbPath, selected, previewRows.length, previewLoadingMore, previewSort, previewTotal]);

  const handlePreviewSort = useCallback(
    async (sort: { column: string; dir: 'asc' | 'desc' } | null) => {
      if (!dbPath || !selected) return;
      setPreviewSort(sort);
      setPreviewLoading(true);
      // Don't clear rows — keep DataGrid mounted so column widths are preserved
      try {
        const result = await getPreview(
          dbPath, selected.schema, selected.table, 200, 0,
          sort?.column, sort?.dir,
        );
        setPreviewColumns(result.columns);
        setPreviewRows(result.rows);
        // Sort doesn't change the row count — keep the cached total.
        setPreviewHasMore(
          previewTotal > 0
            ? result.rows.length < previewTotal
            : result.rows.length === 200
        );
      } catch (err: any) {
        setError(err.message);
      } finally {
        setPreviewLoading(false);
      }
    },
    [dbPath, selected, previewTotal],
  );

  const handleSelectColumn = useCallback(
    (schema: string, table: string, column: string) => {
      if (!dbPath) return;
      setSelected({ schema, table });
      setSelectedColumn({ schema, table, column });
      setActiveTab('column');
    },
    [dbPath],
  );

  // --- Global drag-drop ---
  const [globalDrag, setGlobalDrag] = useState(false);
  const dragCounter = { current: 0 };

  useEffect(() => {
    const onDragEnter = (e: globalThis.DragEvent) => {
      e.preventDefault();
      dragCounter.current++;
      if (dragCounter.current === 1) setGlobalDrag(true);
    };
    const onDragLeave = (e: globalThis.DragEvent) => {
      e.preventDefault();
      dragCounter.current--;
      if (dragCounter.current === 0) setGlobalDrag(false);
    };
    const onDragOver = (e: globalThis.DragEvent) => e.preventDefault();
    const onDrop = (e: globalThis.DragEvent) => {
      e.preventDefault();
      dragCounter.current = 0;
      setGlobalDrag(false);

      const textData = e.dataTransfer?.getData('text/plain');
      if (textData && textData.endsWith('.hyper')) {
        handleOpen(textData);
        return;
      }
      const files = e.dataTransfer?.files;
      if (files && files.length > 0) {
        const file = files[0];
        if (file.name.endsWith('.hyper')) {
          const fullPath = (file as any).path;
          if (fullPath) handleOpen(fullPath);
        }
      }
    };
    document.addEventListener('dragenter', onDragEnter);
    document.addEventListener('dragleave', onDragLeave);
    document.addEventListener('dragover', onDragOver);
    document.addEventListener('drop', onDrop);
    return () => {
      document.removeEventListener('dragenter', onDragEnter);
      document.removeEventListener('dragleave', onDragLeave);
      document.removeEventListener('dragover', onDragOver);
      document.removeEventListener('drop', onDrop);
    };
  }, [handleOpen]);

  const tabs: { key: Tab; label: string }[] = [
    { key: 'preview', label: 'Preview' },
    { key: 'stats', label: 'Column Stats' },
    ...(selectedColumn ? [{ key: 'column' as Tab, label: `${selectedColumn.column}` }] : []),
    { key: 'sql', label: 'SQL Editor' },
  ];

  return (
    <div className="flex flex-col h-screen relative">
      {/* Global drag overlay */}
      {globalDrag && (
        <div className="absolute inset-0 z-50 bg-blue-900/30 border-4 border-dashed border-blue-500 flex items-center justify-center pointer-events-none">
          <div className="bg-gray-900/90 rounded-xl px-8 py-6 text-center">
            <svg className="w-12 h-12 text-blue-400 mx-auto mb-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
            </svg>
            <p className="text-lg text-blue-300 font-medium">Drop .hyper file to open</p>
          </div>
        </div>
      )}

      {/* Header */}
      <header className="bg-gray-900 border-b border-gray-800 px-4 py-3 flex items-center gap-4 shrink-0">
        <h1 className="text-lg font-bold text-blue-400 whitespace-nowrap">Hyper Explorer</h1>
        <FileOpener onOpen={(path) => { setShowGenerator(false); setShowHistory(false); setShowEditor(false); handleOpen(path); }} loading={loading} />
        <button
          onClick={() => { setShowGenerator((v) => !v); setShowHistory(false); setShowEditor(false); }}
          className={`px-3 py-1.5 text-sm rounded transition-colors whitespace-nowrap ${
            showGenerator
              ? 'bg-emerald-600 text-white'
              : 'bg-gray-700 hover:bg-gray-600 text-gray-300'
          }`}
        >
          {showGenerator ? 'Back to Explorer' : 'Generate DB'}
        </button>
        <button
          onClick={() => { setShowHistory((v) => !v); setShowGenerator(false); setShowEditor(false); }}
          className={`px-3 py-1.5 text-sm rounded transition-colors whitespace-nowrap ${
            showHistory
              ? 'bg-purple-600 text-white'
              : 'bg-gray-700 hover:bg-gray-600 text-gray-300'
          }`}
        >
          {showHistory ? 'Back to Explorer' : `Query History${queryHistory.length > 0 ? ` (${queryHistory.length})` : ''}`}
        </button>
        <button
          onClick={() => { setShowEditor((v) => !v); setShowGenerator(false); setShowHistory(false); }}
          disabled={!dbPath}
          className={`px-3 py-1.5 text-sm rounded transition-colors whitespace-nowrap ${
            showEditor
              ? 'bg-blue-600 text-white'
              : dbPath
                ? 'bg-gray-700 hover:bg-gray-600 text-gray-300'
                : 'bg-gray-800 text-gray-600 cursor-not-allowed'
          }`}
        >
          {showEditor ? 'Back to Explorer' : 'Query Editor'}
        </button>
        {dbPath && !showGenerator && !showEditor && (
          <span className="text-xs text-gray-500 truncate" title={dbPath}>
            {dbPath}
          </span>
        )}
      </header>

      {error && (
        <div className="bg-red-900/40 border-b border-red-800 px-4 py-2 text-red-300 text-sm">
          {error}
          <button className="ml-3 text-red-400 hover:text-red-200" onClick={() => setError(null)}>
            dismiss
          </button>
        </div>
      )}

      <div className="flex flex-1 overflow-hidden">
        {showGenerator ? (
          <GenerateDatabase
            onGenerated={(path) => {
              setShowGenerator(false);
              handleOpen(path);
            }}
          />
        ) : showHistory ? (
          <QueryHistory entries={queryHistory} />
        ) : showEditor && dbPath ? (
          <div className="flex-1 flex flex-col overflow-hidden">
            <SqlEditor db={dbPath} schemas={schemas} onQueryExecuted={handleQueryExecuted} />
          </div>
        ) : (
        <>
        {/* Sidebar */}
        <aside className="w-72 bg-gray-900 border-r border-gray-800 overflow-y-auto shrink-0">
          {schemas.length > 0 ? (
            <SchemaTree
              schemas={schemas}
              selected={selected}
              selectedColumn={selectedColumn}
              onSelectTable={handleSelectTable}
              onSelectColumn={handleSelectColumn}
            />
          ) : (
            <div className="p-4 text-gray-500 text-sm">
              {loading ? 'Loading...' : 'Open a .hyper file to explore'}
            </div>
          )}
        </aside>

        {/* Main content */}
        <main className="flex-1 flex flex-col overflow-hidden">
          {selected ? (
            <>
              {/* Tabs */}
              <div className="bg-gray-900 border-b border-gray-800 px-4 flex gap-1 shrink-0">
                {tabs.map((tab) => (
                  <button
                    key={tab.key}
                    onClick={() => setActiveTab(tab.key)}
                    className={`px-4 py-2.5 text-sm font-medium border-b-2 transition-colors ${
                      activeTab === tab.key
                        ? 'border-blue-500 text-blue-400'
                        : 'border-transparent text-gray-400 hover:text-gray-200'
                    }`}
                  >
                    {tab.label}
                  </button>
                ))}
                <span className="ml-auto text-xs text-gray-500 self-center">
                  {selected.schema}.{selected.table}
                </span>
              </div>

              {/* Tab content */}
              <div className="flex-1 overflow-auto">
                {activeTab === 'preview' && (
                  previewColumns.length > 0 ? (
                    <div className="relative flex-1 h-full">
                      {previewLoading && (
                        <div className="absolute inset-0 z-20 bg-gray-900/50 flex items-center justify-center">
                          <LoadingSpinner label="Loading preview..." />
                        </div>
                      )}
                      <DataGrid
                        columns={previewColumns}
                        rows={previewRows}
                        totalRowCount={previewTotal}
                        hasMore={previewHasMore}
                        loadingMore={previewLoadingMore}
                        onLoadMore={handleLoadMorePreview}
                        sort={previewSort}
                        onSort={handlePreviewSort}
                      />
                    </div>
                  ) : previewLoading ? (
                    <LoadingSpinner label="Loading preview..." />
                  ) : null
                )}
                {activeTab === 'stats' && (
                  statsLoading ? (
                    <LoadingSpinner label="Computing statistics..." />
                  ) : statsData ? (
                    <ColumnStats stats={statsData.stats} />
                  ) : null
                )}
                {activeTab === 'column' && dbPath && selectedColumn && (
                  <ColumnDetailView
                    db={dbPath}
                    schema={selectedColumn.schema}
                    table={selectedColumn.table}
                    column={selectedColumn.column}
                  />
                )}
                {activeTab === 'sql' && dbPath && <SqlEditor db={dbPath} schemas={schemas} onQueryExecuted={handleQueryExecuted} />}
              </div>
            </>
          ) : (
            <div className="flex-1 flex items-center justify-center text-gray-600">
              <div className="text-center">
                <p className="text-xl mb-2">Select a table from the sidebar</p>
                <p className="text-sm">or open a .hyper database file first</p>
              </div>
            </div>
          )}
        </main>
        </>
        )}
      </div>
    </div>
  );
}

function LoadingSpinner({ label }: { label: string }) {
  return (
    <div className="flex items-center justify-center p-8 text-gray-400">
      <svg className="animate-spin h-5 w-5 mr-3" viewBox="0 0 24 24">
        <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
        <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
      </svg>
      {label}
    </div>
  );
}
