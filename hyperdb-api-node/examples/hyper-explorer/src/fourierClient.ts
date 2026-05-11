// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

/**
 * Client-side Fourier series computation for interactive term adjustment.
 */

export interface FourierTermClient {
  n: number;
  an: number;
  bn: number;
  amplitude: number;
}

export interface FourierSeriesClient {
  a0: number;
  allTerms: FourierTermClient[]; // sorted by amplitude desc
  termsByFreq: FourierTermClient[]; // sorted by frequency asc
}

/**
 * Precompute all Fourier coefficients from a signal (histogram bucket counts).
 * This is done once; reconstruction with N terms is then instant.
 */
export function computeFourierCoefficients(signal: number[]): FourierSeriesClient {
  const N = signal.length;
  const twoPiOverN = (2 * Math.PI) / N;

  let a0 = 0;
  for (let i = 0; i < N; i++) a0 += signal[i];
  a0 = (2 / N) * a0;

  const allTerms: FourierTermClient[] = [];
  for (let n = 1; n <= Math.floor(N / 2); n++) {
    let an = 0, bn = 0;
    for (let k = 0; k < N; k++) {
      an += signal[k] * Math.cos(twoPiOverN * n * k);
      bn += signal[k] * Math.sin(twoPiOverN * n * k);
    }
    an = (2 / N) * an;
    bn = (2 / N) * bn;
    const amplitude = Math.sqrt(an * an + bn * bn);
    allTerms.push({ n, an, bn, amplitude });
  }

  const termsByFreq = [...allTerms].sort((a, b) => a.n - b.n);
  allTerms.sort((a, b) => b.amplitude - a.amplitude);

  return { a0, allTerms, termsByFreq };
}

/**
 * Find the minimum number of terms (by amplitude) needed so the
 * reconstruction fits within `targetR2` of the original signal.
 *
 * R² = 1 - (SS_res / SS_tot)
 * where SS_res = Σ(original - reconstructed)², SS_tot = Σ(original - mean)²
 */
export function findMinTermsForFit(
  coefs: FourierSeriesClient,
  signal: number[],
  targetR2: number = 0.99,
): number {
  const N = signal.length;
  const mean = signal.reduce((a, b) => a + b, 0) / N;
  const ssTot = signal.reduce((acc, v) => acc + (v - mean) ** 2, 0);
  if (ssTot === 0) return 1; // constant signal

  const twoPiOverN = (2 * Math.PI) / N;
  const maxSearch = coefs.allTerms.length;

  // Binary search: find smallest numTerms where R² >= targetR2
  let lo = 1, hi = maxSearch, best = maxSearch;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const topTerms = coefs.allTerms.slice(0, mid);

    let ssRes = 0;
    for (let k = 0; k < N; k++) {
      let val = coefs.a0 / 2;
      for (const t of topTerms) {
        val += t.an * Math.cos(twoPiOverN * t.n * k) + t.bn * Math.sin(twoPiOverN * t.n * k);
      }
      ssRes += (signal[k] - val) ** 2;
    }

    const r2 = 1 - ssRes / ssTot;
    if (r2 >= targetR2) {
      best = mid;
      hi = mid - 1;
    } else {
      lo = mid + 1;
    }
  }

  return best;
}

/**
 * Compute R² between original signal and reconstructed signal.
 */
export function computeR2(original: number[], reconstructed: number[]): number {
  const N = original.length;
  const mean = original.reduce((a, b) => a + b, 0) / N;
  let ssTot = 0, ssRes = 0;
  for (let i = 0; i < N; i++) {
    ssTot += (original[i] - mean) ** 2;
    ssRes += (original[i] - reconstructed[i]) ** 2;
  }
  if (ssTot === 0) return 1;
  return 1 - ssRes / ssTot;
}

/**
 * Reconstruct the signal using the top `numTerms` harmonics.
 */
export function reconstructWithTerms(
  coefs: FourierSeriesClient,
  signalLength: number,
  numTerms: number,
): { reconstructed: number[]; terms: FourierTermClient[] } {
  const N = signalLength;
  const twoPiOverN = (2 * Math.PI) / N;
  const topTerms = coefs.allTerms.slice(0, numTerms);
  const termsSorted = [...topTerms].sort((a, b) => a.n - b.n);

  const reconstructed: number[] = [];
  for (let k = 0; k < N; k++) {
    let val = coefs.a0 / 2;
    for (const t of topTerms) {
      val += t.an * Math.cos(twoPiOverN * t.n * k) + t.bn * Math.sin(twoPiOverN * t.n * k);
    }
    reconstructed.push(val);
  }

  return { reconstructed, terms: termsSorted };
}
