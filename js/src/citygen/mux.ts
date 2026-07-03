// Minimal WebM muxer for VP9 EncodedVideoChunk payloads. Everything is
// buffered and emitted with explicit sizes (no unknown-size elements): one
// video track, SimpleBlocks only, a new Cluster at each keyframe or when the
// i16 relative timestamp would overflow.

import { parseWebm } from "../webm";

export type MuxPacket = {
  payload: Uint8Array;
  timestampMs: number;
  keyframe: boolean;
};

export type MuxOptions = {
  width: number;
  height: number;
  durationMs: number;
  packets: MuxPacket[];
};

const ID = {
  EBML: 0x1a45dfa3,
  EBMLVersion: 0x4286,
  EBMLReadVersion: 0x42f7,
  EBMLMaxIDLength: 0x42f2,
  EBMLMaxSizeLength: 0x42f3,
  DocType: 0x4282,
  DocTypeVersion: 0x4287,
  DocTypeReadVersion: 0x4285,
  Segment: 0x18538067,
  Info: 0x1549a966,
  TimestampScale: 0x2ad7b1,
  Duration: 0x4489,
  MuxingApp: 0x4d80,
  WritingApp: 0x5741,
  Tracks: 0x1654ae6b,
  TrackEntry: 0xae,
  TrackNumber: 0xd7,
  TrackUID: 0x73c5,
  TrackType: 0x83,
  FlagLacing: 0x9c,
  CodecID: 0x86,
  Video: 0xe0,
  PixelWidth: 0xb0,
  PixelHeight: 0xba,
  Cluster: 0x1f43b675,
  Timestamp: 0xe7,
  SimpleBlock: 0xa3,
} as const;

const APP = "vip9r-citygen";

// Relative block timestamps are i16; stay well inside.
const MAX_RELATIVE_MS = 30_000;

export function muxVp9Webm(options: MuxOptions): Uint8Array {
  const header = element(ID.EBML, [
    uintElement(ID.EBMLVersion, 1),
    uintElement(ID.EBMLReadVersion, 1),
    uintElement(ID.EBMLMaxIDLength, 4),
    uintElement(ID.EBMLMaxSizeLength, 8),
    asciiElement(ID.DocType, "webm"),
    uintElement(ID.DocTypeVersion, 4),
    uintElement(ID.DocTypeReadVersion, 2),
  ]);

  const info = element(ID.Info, [
    uintElement(ID.TimestampScale, 1_000_000),
    floatElement(ID.Duration, options.durationMs),
    asciiElement(ID.MuxingApp, APP),
    asciiElement(ID.WritingApp, APP),
  ]);

  const tracks = element(ID.Tracks, [
    element(ID.TrackEntry, [
      uintElement(ID.TrackNumber, 1),
      uintElement(ID.TrackUID, 1),
      uintElement(ID.TrackType, 1),
      uintElement(ID.FlagLacing, 0),
      asciiElement(ID.CodecID, "V_VP9"),
      element(ID.Video, [
        uintElement(ID.PixelWidth, options.width),
        uintElement(ID.PixelHeight, options.height),
      ]),
    ]),
  ]);

  const clusters: Uint8Array[] = [];
  let blocks: Uint8Array[] = [];
  let clusterTimestampMs = 0;
  const flushCluster = () => {
    if (blocks.length > 0) {
      clusters.push(
        element(ID.Cluster, [
          uintElement(ID.Timestamp, clusterTimestampMs),
          ...blocks,
        ]),
      );
      blocks = [];
    }
  };
  for (const packet of options.packets) {
    if (packet.timestampMs < 0 || !Number.isInteger(packet.timestampMs)) {
      throw new Error(
        `packet timestamp must be a non-negative integer of ms: ${packet.timestampMs}`,
      );
    }
    const relative = packet.timestampMs - clusterTimestampMs;
    if (
      blocks.length === 0 ||
      packet.keyframe ||
      relative > MAX_RELATIVE_MS ||
      relative < 0
    ) {
      flushCluster();
      clusterTimestampMs = packet.timestampMs;
    }
    blocks.push(simpleBlock(packet, clusterTimestampMs));
  }
  flushCluster();

  return concat([header, element(ID.Segment, [info, tracks, ...clusters])]);
}

// Decode the muxed bytes with our own demuxer and check they describe the
// packets that went in. Throws on any mismatch.
export function verifyRoundtrip(webm: Uint8Array, options: MuxOptions): void {
  const parsed = parseWebm(webm);
  if (parsed.width !== options.width || parsed.height !== options.height) {
    throw new Error(`roundtrip dimensions: ${parsed.width}x${parsed.height}`);
  }
  if (parsed.packets.length !== options.packets.length) {
    throw new Error(
      `roundtrip packet count: ${parsed.packets.length} != ${options.packets.length}`,
    );
  }
  for (let index = 0; index < options.packets.length; index += 1) {
    const sent = options.packets[index];
    const got = parsed.packets[index];
    if (
      got.timestamp !== BigInt(sent.timestampMs) ||
      got.keyframe !== sent.keyframe
    ) {
      throw new Error(
        `roundtrip packet ${index}: ts ${got.timestamp} key ${got.keyframe}`,
      );
    }
    if (got.payload.byteLength !== sent.payload.byteLength) {
      throw new Error(
        `roundtrip packet ${index}: payload length ${got.payload.byteLength}`,
      );
    }
  }
}

function simpleBlock(
  packet: MuxPacket,
  clusterTimestampMs: number,
): Uint8Array {
  const relative = packet.timestampMs - clusterTimestampMs;
  const head = new Uint8Array(4);
  head[0] = 0x81; // track 1 as a VINT
  head[1] = (relative >> 8) & 0xff;
  head[2] = relative & 0xff;
  head[3] = packet.keyframe ? 0x80 : 0x00;
  return element(ID.SimpleBlock, [head, packet.payload]);
}

function element(id: number, body: Uint8Array[]): Uint8Array {
  const content = concat(body);
  return concat([encodeId(id), encodeSize(content.byteLength), content]);
}

function uintElement(id: number, value: number): Uint8Array {
  if (value < 0 || !Number.isSafeInteger(value)) {
    throw new Error(`uint element value out of range: ${value}`);
  }
  const bytes: number[] = [];
  let rest = value;
  do {
    bytes.unshift(rest % 256);
    rest = Math.floor(rest / 256);
  } while (rest > 0);
  return concat([
    encodeId(id),
    encodeSize(bytes.length),
    new Uint8Array(bytes),
  ]);
}

function floatElement(id: number, value: number): Uint8Array {
  const body = new Uint8Array(8);
  new DataView(body.buffer).setFloat64(0, value);
  return concat([encodeId(id), encodeSize(8), body]);
}

function asciiElement(id: number, value: string): Uint8Array {
  const body = new TextEncoder().encode(value);
  return concat([encodeId(id), encodeSize(body.byteLength), body]);
}

// EBML IDs are stored verbatim (the marker bit is part of the constant).
function encodeId(id: number): Uint8Array {
  const bytes: number[] = [];
  let rest = id;
  while (rest > 0) {
    bytes.unshift(rest % 256);
    rest = Math.floor(rest / 256);
  }
  return new Uint8Array(bytes);
}

function encodeSize(size: number): Uint8Array {
  let width = 1;
  // The all-ones pattern means "unknown size"; widen before reaching it.
  while (size >= 2 ** (7 * width) - 1) {
    width += 1;
  }
  const out = new Uint8Array(width);
  let rest = size;
  for (let index = width - 1; index >= 0; index -= 1) {
    out[index] = rest % 256;
    rest = Math.floor(rest / 256);
  }
  out[0] |= 0x80 >> (width - 1);
  return out;
}

function concat(parts: Uint8Array[]): Uint8Array {
  let total = 0;
  for (const part of parts) {
    total += part.byteLength;
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.byteLength;
  }
  return out;
}
