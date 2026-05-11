import { useState, useRef, useEffect, type ReactNode } from 'react';
import type { QueryStats } from '../api';

function formatTime(seconds: number | null | undefined): string {
  if (seconds == null) return '—';
  if (seconds < 0.000001) return `${(seconds * 1e9).toFixed(0)} ns`;
  if (seconds < 0.001) return `${(seconds * 1e6).toFixed(1)} µs`;
  if (seconds < 1) return `${(seconds * 1000).toFixed(2)} ms`;
  return `${seconds.toFixed(3)} s`;
}

function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return '—';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

function formatMB(mb: number | null | undefined): string {
  if (mb == null) return '—';
  if (mb < 0.001) return `${(mb * 1024).toFixed(1)} KB`;
  return `${mb.toFixed(3)} MB`;
}

function formatNumber(n: number | null | undefined): string {
  if (n == null) return '—';
  return n.toLocaleString();
}

interface StatRowProps {
  label: string;
  value: string;
  dim?: boolean;
}

function StatRow({ label, value, dim }: StatRowProps) {
  if (value === '—' && dim) return null;
  return (
    <div className="flex justify-between gap-4 text-[11px] leading-relaxed">
      <span className="text-gray-400">{label}</span>
      <span className={`tabular-nums ${value === '—' ? 'text-gray-600' : 'text-gray-200'}`}>{value}</span>
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div>
      <div className="text-[10px] font-semibold text-gray-500 uppercase tracking-wider mb-0.5">{title}</div>
      {children}
    </div>
  );
}

function StatsCard({ stats }: { stats: QueryStats }) {
  const pre = stats.preExecution;
  const exec = stats.execution;

  return (
    <div className="flex flex-col gap-2 min-w-[220px] max-w-[300px]">
      {/* Timing */}
      <Section title="Timing">
        <StatRow label="Total elapsed" value={formatTime(stats.elapsedS)} />
        <StatRow label="Parse" value={formatTime(pre?.parsingTimeS)} dim />
        <StatRow label="Compile" value={formatTime(pre?.compilationTimeS)} dim />
        <StatRow label="Pre-exec total" value={formatTime(pre?.elapsedS)} dim />
        <StatRow label="Execution" value={formatTime(exec?.elapsedS)} />
        <StatRow label="CPU time" value={formatTime(exec?.cpuTimeS)} dim />
        <StatRow label="Thread time" value={formatTime(exec?.threadTimeS)} dim />
        <StatRow label="Wait time" value={formatTime(exec?.waitTimeS)} dim />
        <StatRow label="Schedule" value={formatTime(stats.timeToScheduleS)} dim />
        <StatRow label="Commit" value={formatTime(stats.commitTimeS)} dim />
      </Section>

      {/* Memory */}
      {(pre?.peakMemoryMb != null || exec?.peakMemoryMb != null || stats.peakResultBufferMemoryMb != null) && (
        <Section title="Memory">
          <StatRow label="Pre-exec peak" value={formatMB(pre?.peakMemoryMb)} dim />
          <StatRow label="Exec peak" value={formatMB(exec?.peakMemoryMb)} />
          <StatRow label="Result buffer" value={formatMB(stats.peakResultBufferMemoryMb)} dim />
        </Section>
      )}

      {/* Storage */}
      {(exec?.storageAccessTimeS != null || exec?.storageAccessCount != null) && (
        <Section title="Storage I/O">
          <StatRow label="Access time" value={formatTime(exec?.storageAccessTimeS)} />
          <StatRow label="Accesses" value={formatNumber(exec?.storageAccessCount)} />
          <StatRow label="Bytes read" value={formatBytes(exec?.storageAccessBytes)} />
        </Section>
      )}

      {/* Rows */}
      {(exec?.processedRowsTotal != null || stats.rows != null) && (
        <Section title="Result">
          <StatRow label="Rows processed" value={formatNumber(exec?.processedRowsTotal)} dim />
          <StatRow label="Result rows" value={formatNumber(stats.rows)} dim />
          <StatRow label="Result cols" value={formatNumber(stats.cols)} dim />
          <StatRow label="Result size" value={formatMB(stats.resultSizeMb)} dim />
        </Section>
      )}

      {/* Cache */}
      {stats.planCacheStatus != null && (
        <Section title="Plan Cache">
          <StatRow label="Status" value={stats.planCacheStatus ?? '—'} />
          <StatRow label="Hit count" value={formatNumber(stats.planCacheHitCount)} dim />
        </Section>
      )}
    </div>
  );
}

interface Props {
  stats: QueryStats | null | undefined;
  children: ReactNode;
}

export function QueryStatsTooltip({ stats, children }: Props) {
  const [show, setShow] = useState(false);
  const [position, setPosition] = useState<{ top: number; left: number }>({ top: 0, left: 0 });
  const triggerRef = useRef<HTMLSpanElement>(null);
  const tooltipRef = useRef<HTMLDivElement>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleEnter = () => {
    if (!stats) return;
    timeoutRef.current = setTimeout(() => {
      if (triggerRef.current) {
        const rect = triggerRef.current.getBoundingClientRect();
        setPosition({ top: rect.bottom + 6, left: rect.left });
        setShow(true);
      }
    }, 200);
  };

  const handleLeave = () => {
    if (timeoutRef.current) clearTimeout(timeoutRef.current);
    setShow(false);
  };

  useEffect(() => {
    return () => { if (timeoutRef.current) clearTimeout(timeoutRef.current); };
  }, []);

  // Adjust position if tooltip overflows viewport
  useEffect(() => {
    if (show && tooltipRef.current) {
      const rect = tooltipRef.current.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      let { top, left } = position;
      if (rect.right > vw - 8) left = Math.max(8, vw - rect.width - 8);
      if (rect.bottom > vh - 8) {
        // Show above trigger instead
        if (triggerRef.current) {
          const triggerRect = triggerRef.current.getBoundingClientRect();
          top = triggerRect.top - rect.height - 6;
        }
      }
      if (top !== position.top || left !== position.left) {
        setPosition({ top, left });
      }
    }
  }, [show]);

  if (!stats) {
    return <span>{children}</span>;
  }

  return (
    <>
      <span
        ref={triggerRef}
        onMouseEnter={handleEnter}
        onMouseLeave={handleLeave}
        className="cursor-help border-b border-dotted border-gray-600"
      >
        {children}
      </span>
      {show && (
        <div
          ref={tooltipRef}
          className="fixed z-50 bg-gray-950 border border-gray-700 rounded-lg shadow-2xl px-3 py-2.5"
          style={{ top: position.top, left: position.left }}
          onMouseEnter={() => { if (timeoutRef.current) clearTimeout(timeoutRef.current); }}
          onMouseLeave={handleLeave}
        >
          <StatsCard stats={stats} />
        </div>
      )}
    </>
  );
}
