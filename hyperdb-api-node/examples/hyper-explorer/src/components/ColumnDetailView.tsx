import { useState, useEffect, useCallback, useMemo } from 'react';
import { getColumnDetail, type ColumnDetail, type HistogramBucket, type FFTBin, type FourierSeriesData } from '../api';
import { computeFourierCoefficients, reconstructWithTerms, findMinTermsForFit, computeR2, type FourierSeriesClient } from '../fourierClient';

interface Props {
  db: string;
  schema: string;
  table: string;
  column: string;
}

export function ColumnDetailView({ db, schema, table, column }: Props) {
  const [detail, setDetail] = useState<ColumnDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [numTerms, setNumTerms] = useState(12);

  useEffect(() => {
    setLoading(true);
    setError(null);
    setDetail(null);
    getColumnDetail(db, schema, table, column)
      .then((res) => setDetail(res.detail))
      .catch((err) => setError(err.message))
      .finally(() => setLoading(false));
  }, [db, schema, table, column]);

  const histogram = detail?.histogram;
  const t = detail?.typeName?.toUpperCase() ?? '';
  const isNumeric = t.includes('INT') || t.includes('DOUBLE') || t.includes('FLOAT') || t.includes('NUMERIC') || t.includes('DECIMAL');

  const histogramCounts = useMemo(() => {
    if (!histogram || histogram.length < 2) return null;
    return histogram.map((b) => b.count);
  }, [histogram]);

  const fourierCoefs = useMemo(() => {
    if (!isNumeric || !histogramCounts) return null;
    return computeFourierCoefficients(histogramCounts);
  }, [histogramCounts, isNumeric]);

  const maxTerms = fourierCoefs ? fourierCoefs.allTerms.length : 1;

  // Auto-set slider to minimum terms for 99% R² fit
  useEffect(() => {
    if (fourierCoefs && histogramCounts) {
      const optimal = findMinTermsForFit(fourierCoefs, histogramCounts, 0.99);
      setNumTerms(optimal);
    }
  }, [fourierCoefs, histogramCounts]);

  const optimalTerms = useMemo(() => {
    if (!fourierCoefs || !histogramCounts) return 12;
    return findMinTermsForFit(fourierCoefs, histogramCounts, 0.99);
  }, [fourierCoefs, histogramCounts]);

  const fourierResult = useMemo(() => {
    if (!fourierCoefs || !histogram) return null;
    return reconstructWithTerms(fourierCoefs, histogram.length, numTerms);
  }, [fourierCoefs, histogram, numTerms]);

  const currentR2 = useMemo(() => {
    if (!fourierResult || !histogramCounts) return null;
    return computeR2(histogramCounts, fourierResult.reconstructed);
  }, [fourierResult, histogramCounts]);

  if (loading) {
    return (
      <div className="flex items-center justify-center p-8 text-gray-400">
        <svg className="animate-spin h-5 w-5 mr-3" viewBox="0 0 24 24">
          <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
          <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
        </svg>
        Loading column details...
      </div>
    );
  }

  if (error) {
    return <div className="p-4 text-red-400 text-sm">{error}</div>;
  }

  if (!detail) {
    return <div className="p-4 text-gray-500">No data available</div>;
  }

  const isText = t.includes('TEXT') || t.includes('VARCHAR') || t.includes('CHAR');
  const isBool = t === 'BOOL' || t === 'BOOLEAN';

  return (
    <div className="p-6 max-w-4xl mx-auto space-y-6">
      {/* Header */}
      <div className="flex items-center gap-3">
        <h2 className="text-xl font-bold text-gray-100">{detail.name}</h2>
        <TypeBadge typeName={detail.typeName} />
        <span className="text-sm text-gray-500">{schema}.{table}</span>
      </div>

      {/* Universal stats grid */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        <StatCard label="Rows" value={detail.rowCount.toLocaleString()} />
        <StatCard label="Nulls" value={`${detail.nullCount.toLocaleString()} (${detail.nullPercent}%)`} />
        <StatCard label="Distinct" value={detail.distinctCount.toLocaleString()} />
        <StatCard label="Cardinality" value={`${detail.cardinality}%`} sub="distinct / total" />
      </div>

      {/* Null percentage bar */}
      <div>
        <div className="flex justify-between text-xs text-gray-500 mb-1">
          <span>Null distribution</span>
          <span>{detail.nullPercent}% null</span>
        </div>
        <div className="h-2.5 bg-gray-800 rounded-full overflow-hidden">
          <div
            className="h-full bg-gradient-to-r from-emerald-500 to-emerald-600 rounded-full transition-all"
            style={{ width: `${100 - Math.min(detail.nullPercent, 100)}%` }}
          />
        </div>
        <div className="flex justify-between text-[10px] text-gray-600 mt-0.5">
          <span>non-null</span>
          <span>null</span>
        </div>
      </div>

      {/* Numeric-specific */}
      {isNumeric && (
        <>
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
            <StatCard label="Min" value={fmt(detail.min)} />
            <StatCard label="Max" value={fmt(detail.max)} />
            <StatCard label="Mean" value={fmtFixed(detail.mean)} />
            <StatCard label="Median" value={fmtFixed(detail.median)} />
            <StatCard label="Std Dev" value={fmtFixed(detail.stddev)} />
            <StatCard label="CV" value={detail.cv != null ? `${detail.cv}%` : '—'} sub="stddev / |mean|" />
            <StatCard label="Variance" value={fmtFixed(detail.variance)} />
            <StatCard label="Sum" value={fmt(detail.sum)} />
          </div>

          {/* Percentile box plot */}
          <div>
            <h3 className="text-sm font-semibold text-gray-300 mb-3">Percentile Distribution</h3>
            <PercentileChart
              min={detail.min as number}
              p10={detail.p10 as number}
              p25={detail.p25 as number}
              median={detail.median as number}
              p75={detail.p75 as number}
              p90={detail.p90 as number}
              max={detail.max as number}
            />
          </div>

          {/* Histogram + Fourier series overlay */}
          {detail.histogram && detail.histogram.length > 0 && (
            <div>
              <div className="flex items-center justify-between mb-3">
                <h3 className="text-sm font-semibold text-gray-300">Value Distribution</h3>
                {fourierResult && (
                  <span className="text-[10px] text-gray-600">
                    <span className="inline-block w-3 h-0.5 bg-emerald-400 mr-1 align-middle" /> Fourier series ({numTerms} terms)
                  </span>
                )}
              </div>
              <Histogram buckets={detail.histogram} reconstructed={fourierResult?.reconstructed} />
            </div>
          )}

          {/* Fourier series with slider */}
          {fourierCoefs && fourierResult && (
            <div>
              <div className="flex items-center justify-between mb-2">
                <h3 className="text-sm font-semibold text-gray-300">Fourier Series Approximation</h3>
              </div>

              {/* Term count slider */}
              <div className="bg-gray-900 border border-gray-800 rounded-lg px-4 py-3 mb-3">
                <div className="flex items-center gap-4">
                  <label className="text-xs text-gray-400 whitespace-nowrap">Terms:</label>
                  <input
                    type="range"
                    min={1}
                    max={maxTerms}
                    value={numTerms}
                    onChange={(e) => setNumTerms(parseInt(e.target.value))}
                    className="flex-1 h-1.5 bg-gray-700 rounded-full appearance-none cursor-pointer accent-emerald-500"
                  />
                  <span className="text-sm font-mono text-emerald-400 w-12 text-right">{numTerms}</span>
                  <span className="text-[10px] text-gray-600">/ {maxTerms}</span>
                </div>
                <div className="flex justify-between items-center text-[10px] text-gray-600 mt-1">
                  <span>fewer terms (smoother)</span>
                  <span className="flex items-center gap-2">
                    {currentR2 != null && (
                      <span className={currentR2 >= 0.99 ? 'text-emerald-400' : currentR2 >= 0.95 ? 'text-yellow-400' : 'text-gray-400'}>
                        R² = {(currentR2 * 100).toFixed(2)}%
                      </span>
                    )}
                    <span className="text-gray-600">· 99% fit = {optimalTerms} terms</span>
                  </span>
                  <span>more terms (closer fit)</span>
                </div>
              </div>

              <FourierEquation
                a0={fourierCoefs.a0}
                terms={fourierResult.terms}
                bucketCount={detail.histogram?.length ?? 0}
              />
            </div>
          )}

          {/* FFT of distribution */}
          {detail.fft && detail.fft.length > 0 && (
            <div>
              <h3 className="text-sm font-semibold text-gray-300 mb-1">Frequency Spectrum (FFT)</h3>
              <p className="text-[10px] text-gray-600 mb-3">Fourier transform of the value distribution — reveals periodic structure in the data</p>
              <FFTChart bins={detail.fft} />
            </div>
          )}
        </>
      )}

      {/* Text-specific */}
      {isText && (
        <div className="grid grid-cols-3 gap-3">
          <StatCard label="Min Length" value={fmt(detail.minLength)} />
          <StatCard label="Max Length" value={fmt(detail.maxLength)} />
          <StatCard label="Avg Length" value={fmtFixed(detail.avgLength)} />
        </div>
      )}

      {/* Bool-specific */}
      {isBool && (
        <div>
          <div className="grid grid-cols-3 gap-3 mb-3">
            <StatCard label="True" value={String(detail.trueCount ?? 0)} />
            <StatCard label="False" value={String(detail.falseCount ?? 0)} />
            <StatCard label="True %" value={`${detail.truePercent ?? 0}%`} />
          </div>
          <BoolBar trueCount={detail.trueCount ?? 0} falseCount={detail.falseCount ?? 0} />
        </div>
      )}

      {/* Top values */}
      {detail.topValues && detail.topValues.length > 0 && (
        <div>
          <h3 className="text-sm font-semibold text-gray-300 mb-3">
            Top {detail.topValues.length} Values
          </h3>
          <TopValuesChart values={detail.topValues} totalNonNull={detail.rowCount - detail.nullCount} />
        </div>
      )}
    </div>
  );
}

// =============================================================================
// Sub-components
// =============================================================================

function StatCard({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="bg-gray-900 border border-gray-800 rounded-lg px-3 py-2.5">
      <div className="text-[10px] text-gray-500 uppercase tracking-wide mb-0.5">{label}</div>
      <div className="text-sm font-semibold text-gray-200 truncate" title={value}>{value}</div>
      {sub && <div className="text-[10px] text-gray-600 mt-0.5">{sub}</div>}
    </div>
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
  return <span className={`px-2 py-0.5 rounded text-xs font-medium ${color}`}>{typeName}</span>;
}

function fmt(v: number | string | null | undefined): string {
  if (v == null) return '—';
  if (typeof v === 'number') return v.toLocaleString();
  return String(v);
}

function fmtFixed(v: number | null | undefined, digits = 2): string {
  if (v == null) return '—';
  return Number(v).toLocaleString(undefined, { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

// --- Histogram (smooth area chart) ---

function Histogram({ buckets, reconstructed }: { buckets: HistogramBucket[]; reconstructed?: number[] }) {
  if (buckets.length === 0) return null;

  const maxCount = Math.max(...buckets.map((b) => b.count), ...(reconstructed ?? []), 1);
  const w = 700;
  const h = 200;
  const pad = { top: 12, right: 12, bottom: 32, left: 50 };
  const plotW = w - pad.left - pad.right;
  const plotH = h - pad.top - pad.bottom;

  // Map each bucket midpoint to an (x, y) coordinate
  const points = buckets.map((b, i) => {
    const x = pad.left + ((i + 0.5) / buckets.length) * plotW;
    const y = pad.top + plotH * (1 - b.count / maxCount);
    return { x, y, bucket: b };
  });

  // Build smooth area path using monotone cubic interpolation
  const linePath = buildSmoothPath(points.map((p) => [p.x, p.y]));
  const baseline = pad.top + plotH;
  const areaPath = `M${pad.left},${baseline} L${points[0].x},${points[0].y} ${linePath.slice(linePath.indexOf('C'))} L${points[points.length - 1].x},${baseline} Z`;

  // X-axis label positions (5 evenly spaced)
  const xLabels: { x: number; label: string }[] = [];
  const labelCount = 5;
  for (let i = 0; i < labelCount; i++) {
    const bIdx = Math.round((i / (labelCount - 1)) * (buckets.length - 1));
    const b = buckets[bIdx];
    xLabels.push({
      x: pad.left + ((bIdx + 0.5) / buckets.length) * plotW,
      label: formatAxisValue(i === labelCount - 1 ? b.hi : b.lo),
    });
  }

  return (
    <div className="bg-gray-900 border border-gray-800 rounded-lg p-4 overflow-x-auto">
      <svg viewBox={`0 0 ${w} ${h}`} className="w-full" style={{ minWidth: 400 }}>
        <defs>
          <linearGradient id="histGrad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#3b82f6" stopOpacity={0.5} />
            <stop offset="100%" stopColor="#3b82f6" stopOpacity={0.05} />
          </linearGradient>
        </defs>

        {/* Y axis grid lines */}
        {[0, 0.25, 0.5, 0.75, 1].map((frac) => {
          const y = pad.top + plotH * (1 - frac);
          const label = frac === 0 ? '0' : maxCount >= 1000
            ? `${(maxCount * frac / 1000).toFixed(0)}k`
            : String(Math.round(maxCount * frac));
          return (
            <g key={frac}>
              <line x1={pad.left} x2={w - pad.right} y1={y} y2={y} stroke="#1f2937" strokeWidth={0.5} />
              <text x={pad.left - 6} y={y + 3} textAnchor="end" fill="#4b5563" fontSize={8}>
                {label}
              </text>
            </g>
          );
        })}

        {/* Filled area */}
        <path d={areaPath} fill="url(#histGrad)" />

        {/* Line on top */}
        <path d={linePath} fill="none" stroke="#3b82f6" strokeWidth={1.5} strokeLinejoin="round" />

        {/* Fourier series reconstruction overlay */}
        {reconstructed && reconstructed.length > 0 && (() => {
          const rPoints = reconstructed.map((val, i) => {
            const x = pad.left + ((i + 0.5) / reconstructed.length) * plotW;
            const y = pad.top + plotH * (1 - Math.max(0, val) / maxCount);
            return [x, y];
          });
          const rPath = buildSmoothPath(rPoints);
          return <path d={rPath} fill="none" stroke="#34d399" strokeWidth={2} strokeLinejoin="round" strokeDasharray="4 2" opacity={0.9} />;
        })()}

        {/* X axis labels */}
        {xLabels.map((lbl, i) => (
          <text key={i} x={lbl.x} y={h - 8} fill="#4b5563" fontSize={8} textAnchor="middle">
            {lbl.label}
          </text>
        ))}

        {/* Baseline */}
        <line x1={pad.left} x2={w - pad.right} y1={baseline} y2={baseline} stroke="#374151" strokeWidth={0.5} />
      </svg>
    </div>
  );
}

// Build an SVG path string using monotone cubic spline through points
function buildSmoothPath(pts: number[][]): string {
  if (pts.length < 2) return '';
  if (pts.length === 2) return `M${pts[0][0]},${pts[0][1]} L${pts[1][0]},${pts[1][1]}`;

  let d = `M${pts[0][0]},${pts[0][1]}`;

  // Compute tangents using Fritsch-Carlson monotone method
  const n = pts.length;
  const dx: number[] = [];
  const dy: number[] = [];
  const m: number[] = [];

  for (let i = 0; i < n - 1; i++) {
    dx.push(pts[i + 1][0] - pts[i][0]);
    dy.push(pts[i + 1][1] - pts[i][1]);
    m.push(dy[i] / (dx[i] || 1));
  }

  const tangents: number[] = [m[0]];
  for (let i = 1; i < n - 1; i++) {
    if (m[i - 1] * m[i] <= 0) {
      tangents.push(0);
    } else {
      tangents.push((m[i - 1] + m[i]) / 2);
    }
  }
  tangents.push(m[n - 2]);

  for (let i = 0; i < n - 1; i++) {
    const p0 = pts[i];
    const p1 = pts[i + 1];
    const seg = dx[i] / 3;
    const cp1x = p0[0] + seg;
    const cp1y = p0[1] + tangents[i] * seg;
    const cp2x = p1[0] - seg;
    const cp2y = p1[1] - tangents[i + 1] * seg;
    d += ` C${cp1x},${cp1y} ${cp2x},${cp2y} ${p1[0]},${p1[1]}`;
  }

  return d;
}

function formatAxisValue(v: number): string {
  const abs = Math.abs(v);
  if (abs >= 1e6) return `${(v / 1e6).toFixed(1)}M`;
  if (abs >= 1e3) return `${(v / 1e3).toFixed(1)}k`;
  if (abs < 0.01 && abs > 0) return v.toExponential(1);
  if (Number.isInteger(v)) return v.toLocaleString();
  return v.toFixed(1);
}

// --- FFT frequency spectrum chart ---

function FFTChart({ bins }: { bins: FFTBin[] }) {
  if (bins.length === 0) return null;

  // Show only the first half of frequencies (most informative)
  const displayBins = bins.slice(0, Math.floor(bins.length / 2));
  if (displayBins.length < 2) return null;

  const maxMag = Math.max(...displayBins.map((b) => b.magnitude), 1e-10);
  const w = 700;
  const h = 160;
  const pad = { top: 12, right: 12, bottom: 28, left: 50 };
  const plotW = w - pad.left - pad.right;
  const plotH = h - pad.top - pad.bottom;
  const baseline = pad.top + plotH;

  const points = displayBins.map((b, i) => {
    const x = pad.left + (i / (displayBins.length - 1)) * plotW;
    const y = pad.top + plotH * (1 - b.magnitude / maxMag);
    return { x, y, bin: b };
  });

  const linePath = buildSmoothPath(points.map((p) => [p.x, p.y]));
  const firstC = linePath.indexOf('C');
  const areaPath = firstC >= 0
    ? `M${pad.left},${baseline} L${points[0].x},${points[0].y} ${linePath.slice(firstC)} L${points[points.length - 1].x},${baseline} Z`
    : `M${pad.left},${baseline} ${linePath.replace('M', 'L')} L${points[points.length - 1].x},${baseline} Z`;

  // X labels
  const xLabels: { x: number; label: string }[] = [];
  const labelCount = 5;
  for (let i = 0; i < labelCount; i++) {
    const idx = Math.round((i / (labelCount - 1)) * (displayBins.length - 1));
    xLabels.push({
      x: pad.left + (idx / (displayBins.length - 1)) * plotW,
      label: String(displayBins[idx].frequency),
    });
  }

  return (
    <div className="bg-gray-900 border border-gray-800 rounded-lg p-4 overflow-x-auto">
      <svg viewBox={`0 0 ${w} ${h}`} className="w-full" style={{ minWidth: 400 }}>
        <defs>
          <linearGradient id="fftGrad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#a855f7" stopOpacity={0.45} />
            <stop offset="100%" stopColor="#a855f7" stopOpacity={0.03} />
          </linearGradient>
        </defs>

        {/* Y grid */}
        {[0, 0.25, 0.5, 0.75, 1].map((frac) => {
          const y = pad.top + plotH * (1 - frac);
          return (
            <g key={frac}>
              <line x1={pad.left} x2={w - pad.right} y1={y} y2={y} stroke="#1f2937" strokeWidth={0.5} />
              <text x={pad.left - 6} y={y + 3} textAnchor="end" fill="#4b5563" fontSize={8}>
                {(frac * 100).toFixed(0)}%
              </text>
            </g>
          );
        })}

        {/* Filled area */}
        <path d={areaPath} fill="url(#fftGrad)" />

        {/* Line */}
        <path d={linePath} fill="none" stroke="#a855f7" strokeWidth={1.5} strokeLinejoin="round" />

        {/* X labels */}
        {xLabels.map((lbl, i) => (
          <text key={i} x={lbl.x} y={h - 6} fill="#4b5563" fontSize={8} textAnchor="middle">
            {lbl.label}
          </text>
        ))}

        {/* Axis label */}
        <text x={pad.left + plotW / 2} y={h} fill="#374151" fontSize={7} textAnchor="middle">
          frequency bin
        </text>

        {/* Baseline */}
        <line x1={pad.left} x2={w - pad.right} y1={baseline} y2={baseline} stroke="#374151" strokeWidth={0.5} />
      </svg>
    </div>
  );
}

// --- Fourier series equation display ---

function FourierEquation({ a0, terms, bucketCount }: { a0: number; terms: { n: number; an: number; bn: number; amplitude: number }[]; bucketCount: number }) {
  const N = bucketCount;
  const a0Half = a0 / 2;

  const fmtCoef = (v: number) => {
    if (Math.abs(v) < 0.001) return '0';
    if (Math.abs(v) >= 1000) return v.toFixed(0);
    if (Math.abs(v) >= 10) return v.toFixed(1);
    return v.toFixed(3);
  };

  const termStrings = terms
    .filter((t) => t.amplitude > 0.001)
    .map((t) => {
      const parts: string[] = [];
      if (Math.abs(t.an) >= 0.001) {
        const sign = t.an >= 0 ? '+' : '−';
        parts.push(`${sign} ${fmtCoef(Math.abs(t.an))}·cos(2π·${t.n}·x/${N})`);
      }
      if (Math.abs(t.bn) >= 0.001) {
        const sign = t.bn >= 0 ? '+' : '−';
        parts.push(`${sign} ${fmtCoef(Math.abs(t.bn))}·sin(2π·${t.n}·x/${N})`);
      }
      return parts;
    })
    .flat();

  const maxAmp = terms.length > 0 ? Math.max(...terms.map((t) => t.amplitude)) : 1;

  return (
    <div className="bg-gray-900 border border-gray-800 rounded-lg p-4 space-y-3">
      {/* Compact equation */}
      <div className="font-mono text-xs text-gray-300 leading-relaxed overflow-x-auto whitespace-nowrap">
        <span className="text-emerald-400">f</span>(x) ≈{' '}
        <span className="text-blue-300">{fmtCoef(a0Half)}</span>
        {termStrings.map((s, i) => (
          <span key={i}> {s}</span>
        ))}
      </div>

      {/* Coefficient table */}
      <details className="text-xs">
        <summary className="text-gray-500 cursor-pointer hover:text-gray-300 select-none">
          Show all {terms.length} terms
        </summary>
        <div className="mt-2 overflow-x-auto">
          <table className="text-xs border-collapse w-full">
            <thead>
              <tr className="text-gray-500 border-b border-gray-800">
                <th className="px-2 py-1 text-left font-medium">n</th>
                <th className="px-2 py-1 text-right font-medium">aₙ (cos)</th>
                <th className="px-2 py-1 text-right font-medium">bₙ (sin)</th>
                <th className="px-2 py-1 text-right font-medium">amplitude</th>
              </tr>
            </thead>
            <tbody>
              <tr className="border-b border-gray-800/50 text-gray-400">
                <td className="px-2 py-1">0</td>
                <td className="px-2 py-1 text-right font-mono">{fmtCoef(a0)}</td>
                <td className="px-2 py-1 text-right font-mono">—</td>
                <td className="px-2 py-1 text-right font-mono">{fmtCoef(a0Half)}</td>
              </tr>
              {terms.map((t) => (
                <tr key={t.n} className="border-b border-gray-800/50 text-gray-400">
                  <td className="px-2 py-1">{t.n}</td>
                  <td className="px-2 py-1 text-right font-mono">{fmtCoef(t.an)}</td>
                  <td className="px-2 py-1 text-right font-mono">{fmtCoef(t.bn)}</td>
                  <td className="px-2 py-1 text-right font-mono">
                    <div className="flex items-center justify-end gap-1.5">
                      {fmtCoef(t.amplitude)}
                      <div className="w-16 h-1.5 bg-gray-800 rounded-full overflow-hidden">
                        <div
                          className="h-full bg-emerald-500/60 rounded-full"
                          style={{ width: `${Math.min(100, (t.amplitude / maxAmp) * 100)}%` }}
                        />
                      </div>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>

      <div className="text-[10px] text-gray-600">
        N = {N} buckets · {terms.length} harmonics by amplitude · a₀/2 = {fmtCoef(a0Half)} (DC offset)
      </div>
    </div>
  );
}

// --- Percentile box-plot style chart ---

function PercentileChart({
  min, p10, p25, median, p75, p90, max,
}: {
  min: number; p10: number; p25: number; median: number; p75: number; p90: number; max: number;
}) {
  if (min == null || max == null || min === max) return null;
  const range = max - min;
  const pct = (v: number) => ((v - min) / range) * 100;

  return (
    <div className="bg-gray-900 border border-gray-800 rounded-lg p-4">
      <div className="relative h-10">
        {/* Full range line */}
        <div className="absolute top-1/2 left-0 right-0 h-px bg-gray-700 -translate-y-1/2" />

        {/* Whiskers: p10-p25 and p75-p90 */}
        <div
          className="absolute top-1/2 h-1 bg-gray-600 rounded -translate-y-1/2"
          style={{ left: `${pct(p10)}%`, width: `${pct(p25) - pct(p10)}%` }}
        />
        <div
          className="absolute top-1/2 h-1 bg-gray-600 rounded -translate-y-1/2"
          style={{ left: `${pct(p75)}%`, width: `${pct(p90) - pct(p75)}%` }}
        />

        {/* IQR box: p25-p75 */}
        <div
          className="absolute top-1/2 h-5 bg-blue-600/40 border border-blue-500/60 rounded -translate-y-1/2"
          style={{ left: `${pct(p25)}%`, width: `${pct(p75) - pct(p25)}%` }}
        />

        {/* Median line */}
        <div
          className="absolute top-1/2 w-0.5 h-6 bg-blue-400 -translate-y-1/2"
          style={{ left: `${pct(median)}%` }}
        />

        {/* Min/Max dots */}
        <div
          className="absolute top-1/2 w-1.5 h-1.5 bg-gray-400 rounded-full -translate-y-1/2 -translate-x-1/2"
          style={{ left: `${pct(min)}%` }}
        />
        <div
          className="absolute top-1/2 w-1.5 h-1.5 bg-gray-400 rounded-full -translate-y-1/2 -translate-x-1/2"
          style={{ left: `${pct(max)}%` }}
        />
      </div>

      {/* Labels */}
      <div className="flex justify-between text-[10px] text-gray-500 mt-1">
        <span>min: {min.toLocaleString()}</span>
        <span>p10: {p10.toLocaleString()}</span>
        <span>p25: {p25.toLocaleString()}</span>
        <span className="text-blue-400 font-medium">median: {median.toLocaleString()}</span>
        <span>p75: {p75.toLocaleString()}</span>
        <span>p90: {p90.toLocaleString()}</span>
        <span>max: {max.toLocaleString()}</span>
      </div>
    </div>
  );
}

// --- Bool bar ---

function BoolBar({ trueCount, falseCount }: { trueCount: number; falseCount: number }) {
  const total = trueCount + falseCount;
  if (total === 0) return null;
  const truePct = (trueCount / total) * 100;
  return (
    <div className="bg-gray-900 border border-gray-800 rounded-lg p-3">
      <div className="h-4 flex rounded-full overflow-hidden">
        <div className="bg-emerald-500/70 transition-all" style={{ width: `${truePct}%` }} />
        <div className="bg-red-500/50 transition-all" style={{ width: `${100 - truePct}%` }} />
      </div>
      <div className="flex justify-between text-[10px] text-gray-500 mt-1">
        <span>True ({trueCount})</span>
        <span>False ({falseCount})</span>
      </div>
    </div>
  );
}

// --- Top values horizontal bar chart ---

function TopValuesChart({ values, totalNonNull }: { values: { value: string; count: number }[]; totalNonNull: number }) {
  const maxCount = values.length > 0 ? values[0].count : 1;
  return (
    <div className="bg-gray-900 border border-gray-800 rounded-lg p-4 space-y-1.5">
      {values.map((tv, i) => {
        const pct = totalNonNull > 0 ? (tv.count / totalNonNull) * 100 : 0;
        const barPct = (tv.count / maxCount) * 100;
        return (
          <div key={i} className="flex items-center gap-2">
            <span className="text-xs text-gray-300 w-32 truncate shrink-0" title={tv.value}>
              {tv.value || '(empty)'}
            </span>
            <div className="flex-1 h-4 bg-gray-800 rounded overflow-hidden">
              <div
                className="h-full bg-blue-600/50 rounded transition-all"
                style={{ width: `${barPct}%` }}
              />
            </div>
            <span className="text-[10px] text-gray-500 w-16 text-right shrink-0">
              {tv.count} ({pct.toFixed(1)}%)
            </span>
          </div>
        );
      })}
    </div>
  );
}
