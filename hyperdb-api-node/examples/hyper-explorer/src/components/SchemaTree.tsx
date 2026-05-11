import { useState } from 'react';
import type { SchemaNode } from '../api';

interface Props {
  schemas: SchemaNode[];
  selected: { schema: string; table: string } | null;
  selectedColumn: { schema: string; table: string; column: string } | null;
  onSelectTable: (schema: string, table: string) => void;
  onSelectColumn: (schema: string, table: string, column: string) => void;
}

export function SchemaTree({ schemas, selected, selectedColumn, onSelectTable, onSelectColumn }: Props) {
  const [expanded, setExpanded] = useState<Record<string, boolean>>(() => {
    const init: Record<string, boolean> = {};
    for (const s of schemas) init[s.schema] = true;
    return init;
  });

  const [expandedTables, setExpandedTables] = useState<Record<string, boolean>>({});

  const toggleSchema = (name: string) =>
    setExpanded((prev) => ({ ...prev, [name]: !prev[name] }));

  const toggleTable = (key: string) =>
    setExpandedTables((prev) => ({ ...prev, [key]: !prev[key] }));

  return (
    <div className="py-2">
      {schemas.map((s) => (
        <div key={s.schema}>
          {/* Schema header */}
          <button
            onClick={() => toggleSchema(s.schema)}
            className="w-full flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold text-gray-400 uppercase tracking-wide hover:bg-gray-800"
          >
            <ChevronIcon open={expanded[s.schema]} />
            <span className="text-amber-400/80">{s.schema}</span>
            <span className="ml-auto text-gray-600">{s.tables.length}</span>
          </button>

          {expanded[s.schema] &&
            s.tables.map((t) => {
              const isSelected = selected?.schema === s.schema && selected?.table === t.name;
              const tableKey = `${s.schema}.${t.name}`;
              const showCols = expandedTables[tableKey];

              return (
                <div key={t.name}>
                  <div className="flex items-center">
                    {/* Expand columns toggle */}
                    <button
                      onClick={() => toggleTable(tableKey)}
                      className="pl-5 pr-1 py-1 text-gray-500 hover:text-gray-300"
                    >
                      <ChevronIcon open={showCols} />
                    </button>

                    {/* Table name — click to select */}
                    <button
                      onClick={() => onSelectTable(s.schema, t.name)}
                      className={`flex-1 text-left px-1.5 py-1 text-sm truncate rounded-r transition-colors ${
                        isSelected
                          ? 'bg-blue-600/20 text-blue-300'
                          : 'text-gray-300 hover:bg-gray-800'
                      }`}
                    >
                      {t.name}
                      <span className="ml-1.5 text-xs text-gray-600">({t.columns.length})</span>
                    </button>
                  </div>

                  {/* Column list */}
                  {showCols && (
                    <div className="pl-10 pb-1">
                      {t.columns.map((col) => {
                        const isColSelected =
                          selectedColumn?.schema === s.schema &&
                          selectedColumn?.table === t.name &&
                          selectedColumn?.column === col.name;
                        return (
                          <button
                            key={col.name}
                            onClick={() => onSelectColumn(s.schema, t.name, col.name)}
                            className={`w-full flex items-center gap-2 py-0.5 px-1 text-xs rounded transition-colors ${
                              isColSelected
                                ? 'bg-blue-600/20 text-blue-300'
                                : 'text-gray-500 hover:bg-gray-800 hover:text-gray-300'
                            }`}
                          >
                            <span className={isColSelected ? 'text-blue-300' : 'text-gray-400'}>{col.name}</span>
                            <span className="text-gray-600">{col.typeName}</span>
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              );
            })}
        </div>
      ))}
    </div>
  );
}

function ChevronIcon({ open }: { open?: boolean }) {
  return (
    <svg
      className={`w-3 h-3 transition-transform ${open ? 'rotate-90' : ''}`}
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2}
    >
      <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
    </svg>
  );
}
