import { describe, expect, test } from "vitest";

import { parseWebm, readEbmlVint, WebmDemuxer, type WebmPacket } from ".";
import {
  audioTrack,
  block,
  blockGroup,
  cluster,
  concat,
  element,
  elementWithSize,
  ID,
  intElement,
  segmentContentBytes,
  simpleBlock,
  stringElement,
  trackEntry,
  uintElement,
  unknownSizeElement,
  vp9Track,
  webmFile,
} from "./fixtures";

describe("webm", () => {
  test("reads EBML VINT ids and sizes", () => {
    expect(readEbmlVint(new Uint8Array([0x82]), 0)).toEqual({
      value: 2n,
      width: 1,
      nextOffset: 1,
      unknownSize: false,
    });
    expect(readEbmlVint(new Uint8Array([0x40, 0x7f]), 0).value).toBe(127n);
    expect(
      readEbmlVint(new Uint8Array([0x1a, 0x45, 0xdf, 0xa3]), 0, "id").value,
    ).toBe(0x1a45dfa3n);
    expect(readEbmlVint(new Uint8Array([0xff]), 0).unknownSize).toBe(true);

    expect(() => readEbmlVint(new Uint8Array([0]), 0)).toThrow(
      "invalid EBML VINT at offset 0: first byte is zero",
    );
    expect(() => readEbmlVint(new Uint8Array([0x40]), 0)).toThrow(
      "EBML VINT at offset 0 is truncated",
    );
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
    expect(webm.packets[0].keyframe).toBe(true);
    expect(webm.packets[0].visible).toBe(true);
    expect([...webm.packets[0].payload]).toEqual([1, 2, 3]);
    expect(webm.packets[1].timestamp).toBe(17n);
    expect(webm.packets[1].keyframe).toBe(true);
    expect(webm.packets[1].visible).toBe(true);
    expect([...webm.packets[1].payload]).toEqual([4, 5]);
  });

  test("preserves keyframe and visible block metadata", () => {
    const webm = parseWebm(
      webmFile({
        clusters: [
          cluster(0, [
            simpleBlock(1, 0, 0x88, [1]),
            blockGroup(
              concat(block(1, 1, 0x08, [2]), intElement(ID.ReferenceBlock, -1)),
            ),
          ]),
        ],
      }),
    );

    expect(webm.packets).toHaveLength(2);
    expect(webm.packets[0]).toMatchObject({
      keyframe: true,
      visible: false,
    });
    expect(webm.packets[1]).toMatchObject({
      keyframe: false,
      visible: false,
    });
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

    expect(webm.packets.map((packet) => packet.timestamp)).toEqual([
      102n,
      195n,
    ]);
  });

  test("accepts a complete Segment prefix with a declared size beyond EOF", () => {
    const webm = parseWebm(webmFile({ segmentDeclaredSizeExtra: 10_000 }));

    expect(webm).toMatchObject({
      codecId: "V_VP9",
      width: 320,
      height: 240,
    });
    expect(webm.packets).toHaveLength(1);
    expect([...webm.packets[0].payload]).toEqual([1]);
  });

  test("accepts an unknown-size Segment", () => {
    const bytes = concat(
      element(ID.EBML, stringElement(ID.DocType, "webm")),
      unknownSizeElement(
        ID.Segment,
        segmentContentBytes({
          clusters: [
            cluster(0, [simpleBlock(1, 0, 0x80, [1])]),
            cluster(33, [simpleBlock(1, 0, 0, [2])]),
          ],
        }),
      ),
    );

    const webm = parseWebm(bytes);
    expect(webm.packets.map((packet) => packet.timestamp)).toEqual([0n, 33n]);
  });

  test("rejects malformed and truncated WebM input", () => {
    expect(() => parseWebm(new Uint8Array([0x1a, 0x45, 0xdf]))).toThrow(
      "EBML VINT at offset 0 is truncated",
    );

    const bytes = webmFile();
    expect(() => parseWebm(bytes.subarray(0, bytes.byteLength - 1))).toThrow(
      /size exceeds parent|truncated/,
    );

    expect(() =>
      parseWebm(
        elementWithSize(ID.EBML, 10_000, stringElement(ID.DocType, "webm")),
      ),
    ).toThrow("element 0x1a45dfa3 size exceeds parent");
  });

  test("rejects missing and duplicate VP9 video tracks", () => {
    expect(() =>
      parseWebm(webmFile({ tracks: [audioTrack(1)], clusters: [] })),
    ).toThrow("WebM has no VP9 video track");
    expect(() =>
      parseWebm(
        webmFile({
          tracks: [
            vp9Track(1, { width: 320, height: 240 }),
            vp9Track(2, { width: 320, height: 240 }),
          ],
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
    expect(() =>
      parseWebm(
        webmFile({ clusters: [cluster(0, [simpleBlock(1, 0, 0x02, [0, 1])])] }),
      ),
    ).toThrow("laced VP9 block on track 1 is not supported");
  });
});

describe("webm streaming", () => {
  const clusters = [
    cluster(10, [
      simpleBlock(1, 0, 0x80, [1, 2, 3]),
      blockGroup(block(1, 5, 0, [4, 5])),
    ]),
    cluster(20, [simpleBlock(1, 1, 0, [6])]),
  ];

  test("chunked pushes match the whole-buffer parse, with copied payloads", () => {
    const bytes = webmFile({ clusters });
    const whole = parseWebm(bytes);

    for (const chunkSize of [1, 7, bytes.byteLength]) {
      const demuxer = new WebmDemuxer();
      const packets: WebmPacket[] = [];
      for (let offset = 0; offset < bytes.byteLength; offset += chunkSize) {
        packets.push(
          ...demuxer.push(bytes.subarray(offset, offset + chunkSize)),
        );
      }
      demuxer.finish();

      const { packets: wholePackets, ...header } = whole;
      expect(demuxer.header).toEqual(header);
      expect(packets).toEqual(wholePackets);
      expect(packets[0].payload.buffer).not.toBe(bytes.buffer);
    }
  });

  test("streams an unknown-size Segment and skips non-cluster elements mid-stream", () => {
    const bytes = concat(
      element(ID.EBML, stringElement(ID.DocType, "webm")),
      unknownSizeElement(
        ID.Segment,
        concat(
          segmentContentBytes({ clusters: [clusters[0]] }),
          element(ID.Cues, uintElement(0xbb, 1)),
          clusters[1],
        ),
      ),
    );

    const demuxer = new WebmDemuxer();
    const packets: WebmPacket[] = [];
    for (let offset = 0; offset < bytes.byteLength; offset += 3) {
      packets.push(...demuxer.push(bytes.subarray(offset, offset + 3)));
    }
    demuxer.finish();

    expect(packets.map((packet) => packet.timestamp)).toEqual([10n, 15n, 21n]);
  });

  test("header appears once the first cluster is complete", () => {
    const bytes = webmFile({ clusters });
    const firstClusterEnd = bytes.byteLength - clusters[1].byteLength;

    const demuxer = new WebmDemuxer();
    expect(demuxer.push(bytes.subarray(0, firstClusterEnd - 1))).toEqual([]);
    expect(demuxer.header).toBeUndefined();

    const packets = demuxer.push(
      bytes.subarray(firstClusterEnd - 1, firstClusterEnd),
    );
    expect(demuxer.header).toMatchObject({
      codecId: "V_VP9",
      width: 320,
      height: 240,
    });
    expect(packets).toHaveLength(2);
  });

  test("finish rejects a stream truncated mid-element", () => {
    const bytes = webmFile({ clusters });
    const demuxer = new WebmDemuxer();
    demuxer.push(bytes.subarray(0, bytes.byteLength - 1));
    expect(() => demuxer.finish()).toThrow(/size exceeds parent|truncated/);
  });
});
