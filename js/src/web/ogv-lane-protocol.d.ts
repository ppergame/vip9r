// Wire protocol between bench-worker and ogv-lane-worker. Ambient on purpose:
// the lane worker must stay a classic-worker script, and any module syntax —
// even a type-only import — makes Vite's dev transform append `export {}`,
// which a classic worker cannot parse.

type OgvVideoFormat = {
  width: number;
  height: number;
  chromaWidth: number;
  chromaHeight: number;
  cropLeft: number;
  cropTop: number;
  cropWidth: number;
  cropHeight: number;
  displayWidth: number;
  displayHeight: number;
  fps: number;
};

type OgvPacket = {
  data: Uint8Array;
  timestampUs: number;
};

type OgvLaneInit = {
  moduleScript: string;
  videoFormat: OgvVideoFormat;
  warmupPackets: number;
  packets: OgvPacket[];
};

type OgvLaneEvent =
  | { type: "log"; message: string; error: boolean }
  | { type: "done"; frames: number; wallMs: number }
  | { type: "skipped"; reason: string }
  | { type: "error"; message: string };
