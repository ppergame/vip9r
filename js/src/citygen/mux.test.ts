import { describe, expect, test } from "vitest";

import { parseWebm } from "../webm";
import { muxVp9Webm, verifyRoundtrip } from "./mux";
import type { MuxOptions } from "./mux";

function fakePayload(index: number, length: number): Uint8Array {
  const payload = new Uint8Array(length);
  for (let offset = 0; offset < length; offset += 1) {
    payload[offset] = (index * 31 + offset) & 0xff;
  }
  return payload;
}

describe("citygen mux", () => {
  test("roundtrips through parseWebm", () => {
    const options: MuxOptions = {
      width: 1280,
      height: 720,
      durationMs: 133,
      packets: [
        { payload: fakePayload(0, 900), timestampMs: 0, keyframe: true },
        { payload: fakePayload(1, 120), timestampMs: 33, keyframe: false },
        { payload: fakePayload(2, 140), timestampMs: 67, keyframe: false },
        { payload: fakePayload(3, 950), timestampMs: 100, keyframe: true },
        { payload: fakePayload(4, 110), timestampMs: 133, keyframe: false },
      ],
    };
    const webm = muxVp9Webm(options);

    const parsed = parseWebm(webm);
    expect(parsed.codecId).toBe("V_VP9");
    expect(parsed.width).toBe(1280);
    expect(parsed.height).toBe(720);
    expect(parsed.trackNumber).toBe(1);
    expect(parsed.flagLacing).toBe(0);
    expect(parsed.packets.map((packet) => Number(packet.timestamp))).toEqual([
      0, 33, 67, 100, 133,
    ]);
    expect(parsed.packets.map((packet) => packet.keyframe)).toEqual([
      true,
      false,
      false,
      true,
      false,
    ]);
    expect(parsed.packets.every((packet) => packet.visible)).toBe(true);
    for (const [index, packet] of parsed.packets.entries()) {
      expect(packet.payload).toEqual(options.packets[index].payload);
    }

    verifyRoundtrip(webm, options);
  });

  test("splits clusters before the i16 relative timestamp overflows", () => {
    const packets = [];
    for (let index = 0; index < 40; index += 1) {
      packets.push({
        payload: fakePayload(index, 64),
        timestampMs: index * 1000,
        keyframe: index === 0,
      });
    }
    const options: MuxOptions = {
      width: 320,
      height: 180,
      durationMs: 40_000,
      packets,
    };
    const webm = muxVp9Webm(options);

    const parsed = parseWebm(webm);
    expect(parsed.packets.map((packet) => Number(packet.timestamp))).toEqual(
      packets.map((packet) => packet.timestampMs),
    );
    verifyRoundtrip(webm, options);
  });
});
