// Streaming media source for the web player: fetches a URL and demuxes it
// incrementally, so memory stays bounded by the demux window (one WebM
// cluster / one IVF packet) regardless of clip size. Backpressure is
// pull-driven: the stream is only read when the consumer asks for a packet.
import { IvfDemuxer } from "../ivf";
import { WebmDemuxer } from "../webm";

export type MediaHeader = {
  container: "ivf" | "webm";
  width: number;
  height: number;
  timestampScale?: number;
  timebaseNumerator?: number;
  timebaseDenominator?: number;
};

export type MediaPacket = {
  timestamp: bigint;
  keyframe?: boolean;
  payload: Uint8Array;
};

export type MediaStream = {
  header: MediaHeader;
  packets: AsyncGenerator<MediaPacket, void, void>;
};

type Demuxer = {
  header(): MediaHeader | undefined;
  push(chunk: Uint8Array): MediaPacket[];
  finish(): void;
};

export async function openMediaStream(url: string): Promise<MediaStream> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`media fetch failed: ${response.status} ${response.statusText}`);
  }
  if (response.body === null) {
    throw new Error("media response has no body");
  }
  return demuxMediaBody(response.body);
}

// Split from openMediaStream so tests can feed a synthetic body.
export async function demuxMediaBody(body: ReadableStream<Uint8Array>): Promise<MediaStream> {
  const reader = body.getReader();
  let demuxer: Demuxer | undefined;
  let sniff: Uint8Array = new Uint8Array(0);
  const pending: MediaPacket[] = [];
  let header: MediaHeader | undefined;

  try {
    while (header === undefined) {
      const { done, value } = await reader.read();
      if (done) {
        // Let the demuxer report why the stream is unusable.
        demuxer?.finish();
        throw new Error("media stream ended before demux header");
      }
      if (demuxer === undefined) {
        sniff = concat(sniff, value);
        if (sniff.byteLength < 4) {
          continue;
        }
        demuxer = sniffContainer(sniff);
        pending.push(...demuxer.push(sniff));
      } else {
        pending.push(...demuxer.push(value));
      }
      header = demuxer.header();
    }
  } catch (error) {
    await reader.cancel().catch(() => {});
    throw error;
  }

  const active = demuxer!;
  async function* packets(): AsyncGenerator<MediaPacket, void, void> {
    try {
      for (const packet of pending) {
        yield packet;
      }
      while (true) {
        const { done, value } = await reader.read();
        if (done) {
          active.finish();
          return;
        }
        for (const packet of active.push(value)) {
          yield packet;
        }
      }
    } finally {
      // Also runs when the consumer stops early; drop the connection.
      await reader.cancel().catch(() => {});
    }
  }

  return { header, packets: packets() };
}

function sniffContainer(sniff: Uint8Array): Demuxer {
  if (sniff.byteLength >= 4 && ascii(sniff, 0, 4) === "DKIF") {
    return new IvfStreamDemuxer();
  }
  if (
    sniff.byteLength >= 4 &&
    sniff[0] === 0x1a &&
    sniff[1] === 0x45 &&
    sniff[2] === 0xdf &&
    sniff[3] === 0xa3
  ) {
    return new WebmStreamDemuxer();
  }
  throw new Error("unsupported input container: expected IVF DKIF or WebM EBML");
}

class WebmStreamDemuxer implements Demuxer {
  private readonly demuxer = new WebmDemuxer();

  header(): MediaHeader | undefined {
    const header = this.demuxer.header;
    if (header === undefined) {
      return undefined;
    }
    return {
      container: "webm",
      width: header.width,
      height: header.height,
      timestampScale: header.timestampScale,
    };
  }

  push(chunk: Uint8Array): MediaPacket[] {
    return this.demuxer.push(chunk);
  }

  finish(): void {
    this.demuxer.finish();
  }
}

class IvfStreamDemuxer implements Demuxer {
  private readonly demuxer = new IvfDemuxer();

  header(): MediaHeader | undefined {
    const header = this.demuxer.header;
    if (header === undefined) {
      return undefined;
    }
    return {
      container: "ivf",
      width: header.width,
      height: header.height,
      timebaseDenominator: header.timebaseDenominator,
      timebaseNumerator: header.timebaseNumerator,
    };
  }

  push(chunk: Uint8Array): MediaPacket[] {
    return this.demuxer.push(chunk);
  }

  finish(): void {
    this.demuxer.finish();
  }
}

function ascii(data: Uint8Array, start: number, end: number): string {
  let text = "";
  for (let offset = start; offset < end; offset += 1) {
    text += String.fromCharCode(data[offset]);
  }
  return text;
}

function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.byteLength + b.byteLength);
  out.set(a);
  out.set(b, a.byteLength);
  return out;
}
