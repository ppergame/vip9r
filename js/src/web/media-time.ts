import type { DemuxedVp9 } from "../wasm-driver/golden";

export function packetTimestampUs(input: DemuxedVp9, timestamp: bigint): number {
  if (input.container === "ivf") {
    const numerator = input.timebaseNumerator ?? 1;
    const denominator = input.timebaseDenominator ?? 30;
    return Math.round((Number(timestamp) * 1e6 * numerator) / denominator);
  }
  // WebM: timestamps are in ticks of timestampScale nanoseconds.
  return Math.round((Number(timestamp) * (input.timestampScale ?? 1e6)) / 1000);
}
