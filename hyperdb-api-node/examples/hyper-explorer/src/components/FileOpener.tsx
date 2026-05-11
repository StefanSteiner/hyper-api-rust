import { useState, useCallback, useEffect, type FormEvent, type DragEvent } from 'react';
import { browseDir, type BrowseItem, type BrowseResult } from '../api';

interface Props {
  onOpen: (path: string) => void;
  loading: boolean;
}

export function FileOpener({ onOpen, loading }: Props) {
  const [path, setPath] = useState('');
  const [showBrowser, setShowBrowser] = useState(false);
  const [dragging, setDragging] = useState(false);

  const handleSubmit = useCallback(
    (e: FormEvent) => {
      e.preventDefault();
      const trimmed = path.trim();
      if (trimmed) onOpen(trimmed);
    },
    [path, onOpen],
  );

  // --- Drag and drop ---
  const handleDragOver = useCallback((e: DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragging(true);
  }, []);

  const handleDragLeave = useCallback((e: DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragging(false);
  }, []);

  const handleDrop = useCallback(
    (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setDragging(false);

      // Try to get a file path from the drop event
      // In Electron/Tauri you'd get file.path, but in a web browser
      // we can read the file name. Since this is a local app, we also
      // accept text/plain drops (e.g. from a file manager path bar).
      const textData = e.dataTransfer.getData('text/plain');
      if (textData && textData.endsWith('.hyper')) {
        setPath(textData);
        onOpen(textData);
        return;
      }

      // For actual file drops, browsers give us the file name but not the full path.
      // We'll show the name and let the user know they need to use the file browser.
      const files = e.dataTransfer.files;
      if (files.length > 0) {
        const file = files[0];
        if (file.name.endsWith('.hyper')) {
          // webkitRelativePath or file.path (Electron) may have the full path
          const fullPath = (file as any).path;
          if (fullPath) {
            setPath(fullPath);
            onOpen(fullPath);
          } else {
            // Browser doesn't expose full path — open file browser instead
            setPath(file.name);
            setShowBrowser(true);
          }
        }
      }
    },
    [onOpen],
  );

  return (
    <>
      <form
        onSubmit={handleSubmit}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
        className={`flex gap-2 flex-1 max-w-2xl transition-all ${
          dragging ? 'ring-2 ring-blue-500 ring-offset-1 ring-offset-gray-900 rounded' : ''
        }`}
      >
        <div className="relative flex-1">
          <input
            type="text"
            value={path}
            onChange={(e) => setPath(e.target.value)}
            placeholder={dragging ? 'Drop .hyper file here...' : '/path/to/database.hyper'}
            className={`w-full bg-gray-800 border rounded px-3 py-1.5 text-sm text-gray-200 placeholder-gray-500 focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500 ${
              dragging ? 'border-blue-500 bg-blue-900/20' : 'border-gray-700'
            }`}
            disabled={loading}
          />
        </div>
        <button
          type="button"
          onClick={() => setShowBrowser(true)}
          disabled={loading}
          className="px-3 py-1.5 bg-gray-700 hover:bg-gray-600 disabled:bg-gray-800 disabled:text-gray-600 text-gray-300 text-sm rounded transition-colors"
          title="Browse filesystem"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
          </svg>
        </button>
        <button
          type="submit"
          disabled={loading || !path.trim()}
          className="px-4 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:bg-gray-700 disabled:text-gray-500 text-white text-sm font-medium rounded transition-colors"
        >
          {loading ? 'Opening...' : 'Open'}
        </button>
      </form>

      {showBrowser && (
        <FileBrowserModal
          onSelect={(selectedPath) => {
            setPath(selectedPath);
            setShowBrowser(false);
            onOpen(selectedPath);
          }}
          onClose={() => setShowBrowser(false)}
        />
      )}
    </>
  );
}

// =============================================================================
// File Browser Modal
// =============================================================================

const BROWSE_DIR_KEY = 'hyper-explorer:last-browse-dir';

function FileBrowserModal({
  onSelect,
  onClose,
}: {
  onSelect: (path: string) => void;
  onClose: () => void;
}) {
  const [dir, setDir] = useState('');
  const [items, setItems] = useState<BrowseItem[]>([]);
  const [loadingDir, setLoadingDir] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pathInput, setPathInput] = useState('');

  const loadDir = useCallback(async (targetDir?: string) => {
    setLoadingDir(true);
    setError(null);
    try {
      const result: BrowseResult = await browseDir(targetDir);
      setDir(result.dir);
      setPathInput(result.dir);
      setItems(result.items);
      try { localStorage.setItem(BROWSE_DIR_KEY, result.dir); } catch {}
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoadingDir(false);
    }
  }, []);

  useEffect(() => {
    const lastDir = (() => { try { return localStorage.getItem(BROWSE_DIR_KEY) || undefined; } catch { return undefined; } })();
    loadDir(lastDir);
  }, [loadDir]);

  const handleNavigate = useCallback(
    (item: BrowseItem) => {
      if (item.isDir) {
        loadDir(item.path);
      } else if (item.isHyper) {
        onSelect(item.path);
      }
    },
    [loadDir, onSelect],
  );

  const handlePathSubmit = useCallback(
    (e: FormEvent) => {
      e.preventDefault();
      if (pathInput.trim()) loadDir(pathInput.trim());
    },
    [pathInput, loadDir],
  );

  // Close on Escape
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-50 flex justify-center bg-black/60" style={{ paddingTop: '15vh' }} onClick={onClose}>
      <div
        className="bg-gray-900 border border-gray-700 rounded-lg shadow-2xl w-full max-w-xl flex flex-col self-start"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-800">
          <h2 className="text-sm font-semibold text-gray-200">Browse for .hyper file</h2>
          <button onClick={onClose} className="text-gray-500 hover:text-gray-300">
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Path bar */}
        <form onSubmit={handlePathSubmit} className="px-4 py-2 border-b border-gray-800">
          <div className="flex gap-2">
            <input
              type="text"
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
              className="flex-1 bg-gray-800 border border-gray-700 rounded px-2.5 py-1 text-xs text-gray-300 font-mono focus:outline-none focus:border-blue-500"
              placeholder="/path/to/directory"
            />
            <button
              type="submit"
              className="px-3 py-1 bg-gray-700 hover:bg-gray-600 text-gray-300 text-xs rounded transition-colors"
            >
              Go
            </button>
          </div>
        </form>

        {/* Error */}
        {error && (
          <div className="px-4 py-2 text-xs text-red-400 bg-red-900/20 border-b border-red-900/30">
            {error}
          </div>
        )}

        {/* Fixed ".." parent directory button */}
        {!loadingDir && items.length > 0 && items[0].name === '..' && (
          <button
            onClick={() => handleNavigate(items[0])}
            className="w-full flex items-center gap-2.5 px-4 py-1.5 text-sm text-left hover:bg-gray-800 transition-colors text-gray-300 border-b border-gray-800 shrink-0"
          >
            <svg className="w-4 h-4 text-amber-400/70 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
            </svg>
            <span className="truncate min-w-0 flex-1">..</span>
            <span className="shrink-0 w-36" />
            <span className="shrink-0 w-20" />
          </button>
        )}

        {/* File list — fixed height for ~20 visible rows */}
        <div className="overflow-y-auto" style={{ height: '560px' }}>
          {loadingDir ? (
            <div className="p-4 text-sm text-gray-500">Loading...</div>
          ) : (
            <div className="py-1">
              {items.filter((item) => item.name !== '..').map((item) => (
                <button
                  key={item.path}
                  onClick={() => handleNavigate(item)}
                  onDoubleClick={() => {
                    if (item.isHyper) onSelect(item.path);
                  }}
                  className={`w-full flex items-center gap-2.5 px-4 py-1.5 text-sm text-left hover:bg-gray-800 transition-colors ${
                    item.isHyper ? 'text-blue-300' : 'text-gray-300'
                  }`}
                >
                  {item.isDir ? (
                    <svg className="w-4 h-4 text-amber-400/70 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                    </svg>
                  ) : (
                    <svg className="w-4 h-4 text-blue-400/70 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
                    </svg>
                  )}
                  <span className="truncate min-w-0 flex-1">{item.name}</span>
                  {item.lastModified && (
                    <span className="text-[11px] text-gray-500 shrink-0 w-36 text-right tabular-nums">{formatDate(item.lastModified)}</span>
                  )}
                  {item.size != null && !item.isDir ? (
                    <span className="text-[11px] text-gray-500 shrink-0 w-20 text-right tabular-nums">{formatSize(item.size)}</span>
                  ) : (
                    <span className="shrink-0 w-20" />
                  )}
                </button>
              ))}
              {items.filter((item) => item.name !== '..').length === 0 && (
                <div className="px-4 py-3 text-sm text-gray-600">
                  No directories or .hyper files found
                </div>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="px-4 py-2 border-t border-gray-800 text-xs text-gray-600">
          Click a .hyper file to open it, or navigate into directories
        </div>
      </div>
    </div>
  );
}

// =============================================================================
// Formatting helpers
// =============================================================================

function formatDate(iso: string): string {
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}
