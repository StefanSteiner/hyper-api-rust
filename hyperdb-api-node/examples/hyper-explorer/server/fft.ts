// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

/**
 * Fourier series coefficients and reconstruction.
 */

export interface FourierTerm {
  n: number;
  an: number;
  bn: number;
  amplitude: number;
  phase: number;
}

export interface FourierSeriesResult {
  a0: number;
  terms: FourierTerm[];
  reconstructed: number[];  // same length as input
}

/**
 * Compute real Fourier series of a discrete signal.
 * Returns a0 (DC), the top `numTerms` harmonics (by amplitude), and the
 * reconstructed curve using those terms.
 *
 * f(x) ≈ a0/2 + Σ [an·cos(2πnx/N) + bn·sin(2πnx/N)]
 */
export function fourierSeries(signal: number[], numTerms: number = 12): FourierSeriesResult {
  const N = signal.length;
  const twoPiOverN = (2 * Math.PI) / N;

  // Compute all Fourier coefficients via DFT
  const allTerms: FourierTerm[] = [];
  let a0 = 0;
  for (let i = 0; i < N; i++) a0 += signal[i];
  a0 = (2 / N) * a0;

  for (let n = 1; n <= Math.floor(N / 2); n++) {
    let an = 0, bn = 0;
    for (let k = 0; k < N; k++) {
      an += signal[k] * Math.cos(twoPiOverN * n * k);
      bn += signal[k] * Math.sin(twoPiOverN * n * k);
    }
    an = (2 / N) * an;
    bn = (2 / N) * bn;
    const amplitude = Math.sqrt(an * an + bn * bn);
    const phase = Math.atan2(bn, an);
    allTerms.push({ n, an, bn, amplitude, phase });
  }

  // Keep top `numTerms` by amplitude
  allTerms.sort((a, b) => b.amplitude - a.amplitude);
  const topTerms = allTerms.slice(0, numTerms);
  topTerms.sort((a, b) => a.n - b.n); // re-sort by frequency for display

  // Reconstruct using selected terms
  const reconstructed: number[] = [];
  for (let k = 0; k < N; k++) {
    let val = a0 / 2;
    for (const t of topTerms) {
      val += t.an * Math.cos(twoPiOverN * t.n * k) + t.bn * Math.sin(twoPiOverN * t.n * k);
    }
    reconstructed.push(val);
  }

  return { a0, terms: topTerms, reconstructed };
}

/**
 * Radix-2 Cooley-Tukey FFT (in-place, iterative).
 * Input length must be a power of 2 — pad with zeros if needed.
 *
 * Returns magnitude spectrum (first half only — symmetric for real input).
 */

export function fftMagnitude(signal: number[]): number[] {
  // Pad to next power of 2
  let n = 1;
  while (n < signal.length) n <<= 1;
  const re = new Float64Array(n);
  const im = new Float64Array(n);
  for (let i = 0; i < signal.length; i++) re[i] = signal[i];

  // Bit-reversal permutation
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) {
      j ^= bit;
    }
    j ^= bit;
    if (i < j) {
      [re[i], re[j]] = [re[j], re[i]];
      [im[i], im[j]] = [im[j], im[i]];
    }
  }

  // FFT butterfly
  for (let len = 2; len <= n; len <<= 1) {
    const halfLen = len >> 1;
    const angle = (-2 * Math.PI) / len;
    const wRe = Math.cos(angle);
    const wIm = Math.sin(angle);
    for (let i = 0; i < n; i += len) {
      let curRe = 1, curIm = 0;
      for (let j = 0; j < halfLen; j++) {
        const uRe = re[i + j];
        const uIm = im[i + j];
        const vRe = re[i + j + halfLen] * curRe - im[i + j + halfLen] * curIm;
        const vIm = re[i + j + halfLen] * curIm + im[i + j + halfLen] * curRe;
        re[i + j] = uRe + vRe;
        im[i + j] = uIm + vIm;
        re[i + j + halfLen] = uRe - vRe;
        im[i + j + halfLen] = uIm - vIm;
        const newCurRe = curRe * wRe - curIm * wIm;
        curIm = curRe * wIm + curIm * wRe;
        curRe = newCurRe;
      }
    }
  }

  // Return magnitude of first half (DC excluded, up to Nyquist)
  const halfN = n >> 1;
  const magnitudes: number[] = [];
  for (let i = 1; i <= halfN; i++) {
    magnitudes.push(Math.sqrt(re[i] * re[i] + im[i] * im[i]) / n);
  }
  return magnitudes;
}
