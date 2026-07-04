export type MediaTimebase = {
  container: "ivf" | "webm";
  timestampScale?: number;
  timebaseNumerator?: number;
  timebaseDenominator?: number;
};

export function packetTimestampUs(input: MediaTimebase, timestamp: bigint): number {
  if (input.container === "ivf") {
    const numerator = input.timebaseNumerator ?? 1;
    const denominator = input.timebaseDenominator ?? 30;
    return Math.round((Number(timestamp) * 1e6 * numerator) / denominator);
  }
  // WebM: timestamps are in ticks of timestampScale nanoseconds.
  return Math.round((Number(timestamp) * (input.timestampScale ?? 1e6)) / 1000);
}
