// Incremental IVF demuxer. push() accepts stream chunks and returns the
// packets completed by them; memory is bounded by the demux window (one
// packet). `header` becomes available once the fixed 32-byte header is
// buffered. parseIvf() wraps it for whole-buffer input.

export type IvfHeader = {
  fourcc: "VP90";
  width: number;
  height: number;
  timebaseDenominator: number;
  timebaseNumerator: number;
  declaredFrameCount: number;
};

export type IvfPacket = {
  index: number;
  timestamp: bigint;
  payload: Uint8Array;
};

export type IvfFile = IvfHeader & {
  packets: IvfPacket[];
};

export function parseIvf(data: Uint8Array): IvfFile {
  const demuxer = new IvfDemuxer();
  const packets = demuxer.push(data);
  demuxer.finish();
  // finish() rejects packet-less streams, and packets imply a parsed header.
  return { ...demuxer.header!, packets };
}

export class IvfDemuxer {
  private data: Uint8Array = new Uint8Array(0);
  // Parse position within `data`; everything before it is consumed.
  private cursor = 0;
  private headerValue: IvfHeader | undefined;
  private headerLength = 0;
  // IVF header bytes beyond the fixed 32 still to discard.
  private skipRemaining = 0;
  private packetCount = 0;

  get header(): IvfHeader | undefined {
    return this.headerValue;
  }

  push(chunk: Uint8Array): IvfPacket[] {
    this.append(chunk);
    const out: IvfPacket[] = [];
    if (this.headerValue === undefined) {
      if (this.data.byteLength - this.cursor < 32) {
        return out;
      }
      this.parseHeader();
    }
    if (this.skipRemaining > 0) {
      const take = Math.min(
        this.data.byteLength - this.cursor,
        this.skipRemaining,
      );
      this.cursor += take;
      this.skipRemaining -= take;
      if (this.skipRemaining > 0) {
        return out;
      }
    }
    while (true) {
      const avail = this.data.byteLength - this.cursor;
      if (avail < 12) {
        break;
      }
      const length = le32(this.data, this.cursor);
      if (avail < 12 + length) {
        break;
      }
      out.push({
        index: this.packetCount,
        timestamp: le64(this.data, this.cursor + 4),
        payload: this.data.slice(this.cursor + 12, this.cursor + 12 + length),
      });
      this.cursor += 12 + length;
      this.packetCount += 1;
    }
    return out;
  }

  finish(): void {
    if (this.headerValue === undefined) {
      throw new Error("IVF header is truncated");
    }
    if (this.skipRemaining > 0) {
      throw new Error(
        `IVF header length exceeds file size: ${this.headerLength}`,
      );
    }
    const avail = this.data.byteLength - this.cursor;
    if (avail > 0) {
      throw new Error(
        avail < 12
          ? `packet ${this.packetCount} header is truncated`
          : `packet ${this.packetCount} payload is truncated`,
      );
    }
    if (this.packetCount === 0) {
      throw new Error("IVF contains no packets");
    }
  }

  private parseHeader(): void {
    const data = this.data;
    const base = this.cursor;
    if (ascii(data, base, base + 4) !== "DKIF") {
      throw new Error("IVF signature is not DKIF");
    }
    const version = le16(data, base + 4);
    if (version !== 0) {
      throw new Error(`unsupported IVF version: ${version}`);
    }
    this.headerLength = le16(data, base + 6);
    if (this.headerLength < 32) {
      throw new Error(`IVF header length is too small: ${this.headerLength}`);
    }
    const fourcc = ascii(data, base + 8, base + 12);
    if (fourcc !== "VP90") {
      throw new Error(`unsupported IVF fourcc: ${fourcc}`);
    }
    this.headerValue = {
      fourcc,
      width: le16(data, base + 12),
      height: le16(data, base + 14),
      timebaseDenominator: le32(data, base + 16),
      timebaseNumerator: le32(data, base + 20),
      declaredFrameCount: le32(data, base + 24),
    };
    this.cursor = base + 32;
    this.skipRemaining = this.headerLength - 32;
  }

  private append(chunk: Uint8Array): void {
    if (this.cursor === this.data.byteLength) {
      this.data = chunk;
      this.cursor = 0;
      return;
    }
    const tail = this.data.subarray(this.cursor);
    const next = new Uint8Array(tail.byteLength + chunk.byteLength);
    next.set(tail);
    next.set(chunk, tail.byteLength);
    this.data = next;
    this.cursor = 0;
  }
}

function ascii(data: Uint8Array, start: number, end: number): string {
  let text = "";
  for (let offset = start; offset < end; offset += 1) {
    text += String.fromCharCode(data[offset]);
  }
  return text;
}

function le16(data: Uint8Array, offset: number): number {
  return data[offset] | (data[offset + 1] << 8);
}

function le32(data: Uint8Array, offset: number): number {
  return (
    (data[offset] |
      (data[offset + 1] << 8) |
      (data[offset + 2] << 16) |
      (data[offset + 3] << 24)) >>>
    0
  );
}

function le64(data: Uint8Array, offset: number): bigint {
  return BigInt(le32(data, offset)) | (BigInt(le32(data, offset + 4)) << 32n);
}
