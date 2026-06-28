import { describe, expect, test } from "vitest";

import { parseWebm, readEbmlVint } from ".";

const ID = {
  EBML: 0x1a45dfa3,
  DocType: 0x4282,
  Segment: 0x18538067,
  Info: 0x1549a966,
  TimestampScale: 0x2ad7b1,
  Tracks: 0x1654ae6b,
  TrackEntry: 0xae,
  TrackNumber: 0xd7,
  TrackType: 0x83,
  FlagLacing: 0x9c,
  DefaultDuration: 0x23e383,
  CodecID: 0x86,
  Video: 0xe0,
  PixelWidth: 0xb0,
  PixelHeight: 0xba,
  Cluster: 0x1f43b675,
  Timestamp: 0xe7,
  SimpleBlock: 0xa3,
  BlockGroup: 0xa0,
  Block: 0xa1,
  Cues: 0x1c53bb6b,
} as const;

describe("webm", () => {
  test("reads EBML VINT ids and sizes", () => {
    expect(readEbmlVint(new Uint8Array([0x82]), 0)).toEqual({
      value: 2n,
      width: 1,
      nextOffset: 1,
      unknownSize: false,
    });
    expect(readEbmlVint(new Uint8Array([0x40, 0x7f]), 0).value).toBe(127n);
    expect(readEbmlVint(new Uint8Array([0x1a, 0x45, 0xdf, 0xa3]), 0, "id").value).toBe(0x1a45dfa3n);
    expect(readEbmlVint(new Uint8Array([0xff]), 0).unknownSize).toBe(true);

    expect(() => readEbmlVint(new Uint8Array([0]), 0)).toThrow(
      "invalid EBML VINT at offset 0: first byte is zero",
    );
    expect(() => readEbmlVint(new Uint8Array([0x40]), 0)).toThrow("EBML VINT at offset 0 is truncated");
  });

  test("parses a VP9 WebM, skips audio and Cues, and preserves packet payload subarrays", () => {
    const bytes = webmFile({
      tracks: [vp9Track(1, { width: 320, height: 240 }), audioTrack(2)],
      segmentExtras: [element(ID.Cues, uintElement(0xbb, 1))],
      clusters: [
        cluster(10, [
          simpleBlock(2, 0, 0, [99]),
          simpleBlock(1, 5, 0x80, [1, 2, 3]),
          blockGroup(block(1, 7, 0, [4, 5])),
        ]),
      ],
    });

    const webm = parseWebm(bytes);

    expect(webm).toMatchObject({
      codecId: "V_VP9",
      width: 320,
      height: 240,
      timestampScale: 1_000_000,
      trackNumber: 1,
      flagLacing: 0,
      defaultDuration: 33_333_333,
    });
    expect(webm.packets).toHaveLength(2);
    expect(webm.packets[0].timestamp).toBe(15n);
    expect([...webm.packets[0].payload]).toEqual([1, 2, 3]);
    expect(webm.packets[0].payload.buffer).toBe(bytes.buffer);
    expect(webm.packets[1].timestamp).toBe(17n);
    expect([...webm.packets[1].payload]).toEqual([4, 5]);
  });

  test("adds cluster timestamp and signed block timestamp", () => {
    const webm = parseWebm(
      webmFile({
        clusters: [
          cluster(100, [simpleBlock(1, 2, 0, [1])]),
          cluster(200, [simpleBlock(1, -5, 0, [2])]),
        ],
      }),
    );

    expect(webm.packets.map((packet) => packet.timestamp)).toEqual([102n, 195n]);
  });

  test("rejects malformed and truncated WebM input", () => {
    expect(() => parseWebm(new Uint8Array([0x1a, 0x45, 0xdf]))).toThrow("EBML VINT at offset 0 is truncated");

    const bytes = webmFile();
    expect(() => parseWebm(bytes.subarray(0, bytes.byteLength - 1))).toThrow(/size exceeds parent|truncated/);
  });

  test("rejects missing and duplicate VP9 video tracks", () => {
    expect(() => parseWebm(webmFile({ tracks: [audioTrack(1)], clusters: [] }))).toThrow(
      "WebM has no VP9 video track",
    );
    expect(() =>
      parseWebm(
        webmFile({
          tracks: [vp9Track(1, { width: 320, height: 240 }), vp9Track(2, { width: 320, height: 240 })],
          clusters: [],
        }),
      ),
    ).toThrow("WebM has multiple VP9 video tracks");
  });

  test("rejects missing VP9 dimensions", () => {
    expect(() =>
      parseWebm(
        webmFile({
          tracks: [
            trackEntry([
              uintElement(ID.TrackNumber, 1),
              uintElement(ID.TrackType, 1),
              stringElement(ID.CodecID, "V_VP9"),
            ]),
          ],
          clusters: [],
        }),
      ),
    ).toThrow("VP9 track is missing Video/PixelWidth");
  });

  test("rejects laced selected-video blocks", () => {
    expect(() => parseWebm(webmFile({ clusters: [cluster(0, [simpleBlock(1, 0, 0x02, [0, 1])])] }))).toThrow(
      "laced VP9 block on track 1 is not supported",
    );
  });
});

function webmFile(
  options: {
    tracks?: Uint8Array[];
    segmentExtras?: Uint8Array[];
    clusters?: Uint8Array[];
  } = {},
): Uint8Array {
  const tracks = options.tracks ?? [vp9Track(1, { width: 320, height: 240 })];
  const clusters = options.clusters ?? [cluster(0, [simpleBlock(1, 0, 0, [1])])];
  return concat(
    element(ID.EBML, stringElement(ID.DocType, "webm")),
    element(
      ID.Segment,
      concat(
        element(ID.Info, uintElement(ID.TimestampScale, 1_000_000)),
        element(ID.Tracks, concat(...tracks)),
        ...(options.segmentExtras ?? []),
        ...clusters,
      ),
    ),
  );
}

function vp9Track(trackNumber: number, dimensions: { width: number; height: number }): Uint8Array {
  return trackEntry([
    uintElement(ID.TrackNumber, trackNumber),
    uintElement(ID.TrackType, 1),
    stringElement(ID.CodecID, "V_VP9"),
    uintElement(ID.FlagLacing, 0),
    uintElement(ID.DefaultDuration, 33_333_333),
    element(
      ID.Video,
      concat(uintElement(ID.PixelWidth, dimensions.width), uintElement(ID.PixelHeight, dimensions.height)),
    ),
  ]);
}

function audioTrack(trackNumber: number): Uint8Array {
  return trackEntry([
    uintElement(ID.TrackNumber, trackNumber),
    uintElement(ID.TrackType, 2),
    stringElement(ID.CodecID, "A_OPUS"),
  ]);
}

function trackEntry(fields: Uint8Array[]): Uint8Array {
  return element(ID.TrackEntry, concat(...fields));
}

function cluster(timestamp: number, blocks: Uint8Array[]): Uint8Array {
  return element(ID.Cluster, concat(uintElement(ID.Timestamp, timestamp), ...blocks));
}

function simpleBlock(trackNumber: number, relativeTimestamp: number, flags: number, payload: number[]): Uint8Array {
  return element(ID.SimpleBlock, blockContent(trackNumber, relativeTimestamp, flags, payload));
}

function block(trackNumber: number, relativeTimestamp: number, flags: number, payload: number[]): Uint8Array {
  return element(ID.Block, blockContent(trackNumber, relativeTimestamp, flags, payload));
}

function blockGroup(blockElement: Uint8Array): Uint8Array {
  return element(ID.BlockGroup, blockElement);
}

function blockContent(trackNumber: number, relativeTimestamp: number, flags: number, payload: number[]): Uint8Array {
  if (trackNumber < 1 || trackNumber > 126) {
    throw new Error(`test fixture track number is out of range: ${trackNumber}`);
  }
  const timestamp = relativeTimestamp < 0 ? 0x1_0000 + relativeTimestamp : relativeTimestamp;
  return new Uint8Array([0x80 | trackNumber, (timestamp >>> 8) & 0xff, timestamp & 0xff, flags, ...payload]);
}

function element(id: number, content: Uint8Array): Uint8Array {
  return concat(new Uint8Array(idBytes(id)), new Uint8Array(sizeVint(content.byteLength)), content);
}

function uintElement(id: number, value: number): Uint8Array {
  return element(id, new Uint8Array(uintBytes(value)));
}

function stringElement(id: number, value: string): Uint8Array {
  const bytes: number[] = [];
  for (let index = 0; index < value.length; index += 1) {
    bytes.push(value.charCodeAt(index));
  }
  return element(id, new Uint8Array(bytes));
}

function idBytes(id: number): number[] {
  const bytes: number[] = [];
  let started = false;
  for (let shift = 24; shift >= 0; shift -= 8) {
    const byte = (id >>> shift) & 0xff;
    if (byte !== 0 || started) {
      bytes.push(byte);
      started = true;
    }
  }
  return bytes;
}

function sizeVint(size: number): number[] {
  if (size <= 0x7e) {
    return [0x80 | size];
  }
  if (size <= 0x3ffe) {
    return [0x40 | (size >>> 8), size & 0xff];
  }
  if (size <= 0x0fff_fffe) {
    return [0x10 | ((size >>> 24) & 0x0f), (size >>> 16) & 0xff, (size >>> 8) & 0xff, size & 0xff];
  }
  throw new Error(`test fixture element is too large: ${size}`);
}

function uintBytes(value: number): number[] {
  if (value === 0) {
    return [0];
  }
  const bytes: number[] = [];
  let remaining = value;
  while (remaining > 0) {
    bytes.unshift(remaining & 0xff);
    remaining = Math.floor(remaining / 0x100);
  }
  return bytes;
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const length = parts.reduce((sum, part) => sum + part.byteLength, 0);
  const out = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.byteLength;
  }
  return out;
}
