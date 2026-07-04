import { describe, expect, test } from "vitest";

import { parseWebm } from "../webm";
import { cluster, simpleBlock, webmFile } from "../webm/fixtures";
import { demuxMediaBody } from "./media-stream";
import type { MediaPacket, MediaStream } from "./media-stream";

const WEBM_BYTES = webmFile({
  clusters: [
    cluster(10, [simpleBlock(1, 0, 0x80, [1, 2, 3])]),
    cluster(20, [simpleBlock(1, 1, 0, [4, 5])]),
  ],
});

function bodyFromChunks(bytes: Uint8Array, chunkSize: number): ReadableStream<Uint8Array> {
  let offset = 0;
  return new ReadableStream({
    pull(controller) {
      if (offset >= bytes.byteLength) {
        controller.close();
        return;
      }
      controller.enqueue(bytes.slice(offset, offset + chunkSize));
      offset += chunkSize;
    },
  });
}

async function collect(media: MediaStream): Promise<MediaPacket[]> {
  const packets: MediaPacket[] = [];
  for await (const packet of media.packets) {
    packets.push(packet);
  }
  return packets;
}

function ivfFile(packets: { timestamp: bigint; payload: number[] }[]): Uint8Array {
  const size = 32 + packets.reduce((sum, packet) => sum + 12 + packet.payload.length, 0);
  const bytes = new Uint8Array(size);
  const view = new DataView(bytes.buffer);
  bytes.set([0x44, 0x4b, 0x49, 0x46]); // DKIF
  view.setUint16(4, 0, true); // version
  view.setUint16(6, 32, true); // header length
  bytes.set([0x56, 0x50, 0x39, 0x30], 8); // VP90
  view.setUint16(12, 320, true);
  view.setUint16(14, 240, true);
  view.setUint32(16, 30, true); // timebase denominator
  view.setUint32(20, 1, true); // timebase numerator
  view.setUint32(24, packets.length, true);
  let offset = 32;
  for (const packet of packets) {
    view.setUint32(offset, packet.payload.length, true);
    view.setBigUint64(offset + 4, packet.timestamp, true);
    bytes.set(packet.payload, offset + 12);
    offset += 12 + packet.payload.length;
  }
  return bytes;
}

describe("media-stream", () => {
  test("demuxes chunked WebM to the whole-buffer parse result", async () => {
    const whole = parseWebm(WEBM_BYTES);
    for (const chunkSize of [1, 7, WEBM_BYTES.byteLength]) {
      const media = await demuxMediaBody(bodyFromChunks(WEBM_BYTES, chunkSize));
      expect(media.header).toEqual({
        container: "webm",
        width: 320,
        height: 240,
        timestampScale: 1_000_000,
      });
      const packets = await collect(media);
      expect(packets.map((packet) => packet.timestamp)).toEqual(
        whole.packets.map((packet) => packet.timestamp),
      );
      expect(packets.map((packet) => [...packet.payload])).toEqual(
        whole.packets.map((packet) => [...packet.payload]),
      );
    }
  });

  test("demuxes chunked IVF", async () => {
    const bytes = ivfFile([
      { timestamp: 0n, payload: [1, 2, 3] },
      { timestamp: 1n, payload: [4] },
    ]);
    for (const chunkSize of [1, 5, bytes.byteLength]) {
      const media = await demuxMediaBody(bodyFromChunks(bytes, chunkSize));
      expect(media.header).toEqual({
        container: "ivf",
        width: 320,
        height: 240,
        timebaseDenominator: 30,
        timebaseNumerator: 1,
      });
      const packets = await collect(media);
      expect(packets.map((packet) => packet.timestamp)).toEqual([0n, 1n]);
      expect(packets.map((packet) => [...packet.payload])).toEqual([[1, 2, 3], [4]]);
    }
  });

  test("cancels the body when the consumer stops early", async () => {
    let cancelled = false;
    let offset = 0;
    const body = new ReadableStream<Uint8Array>({
      pull(controller) {
        // Never closes: simulates a large remote file.
        controller.enqueue(WEBM_BYTES.slice(offset, offset + 16));
        offset += 16;
      },
      cancel() {
        cancelled = true;
      },
    });

    const media = await demuxMediaBody(body);
    const first = await media.packets.next();
    expect(first.done).toBe(false);
    await media.packets.return();
    expect(cancelled).toBe(true);
  });

  test("rejects unknown containers and packet-less streams", async () => {
    await expect(demuxMediaBody(bodyFromChunks(new Uint8Array([1, 2, 3, 4, 5]), 2))).rejects.toThrow(
      "unsupported input container",
    );
    const noClusters = webmFile({ clusters: [] });
    await expect(demuxMediaBody(bodyFromChunks(noClusters, 8))).rejects.toThrow(
      "WebM contains no VP9 packets",
    );
    const truncated = WEBM_BYTES.subarray(0, WEBM_BYTES.byteLength - 1);
    const media = await demuxMediaBody(bodyFromChunks(truncated, 16));
    await expect(collect(media)).rejects.toThrow(/size exceeds parent|truncated/);
  });
});
