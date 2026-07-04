export type WebmPacket = {
  index: number;
  timestamp: bigint;
  keyframe: boolean;
  visible: boolean;
  payload: Uint8Array;
};

export type WebmHeader = {
  codecId: "V_VP9";
  width: number;
  height: number;
  timestampScale: number;
  trackNumber: number;
  flagLacing: number;
  defaultDuration?: number;
};

export type WebmFile = WebmHeader & {
  packets: WebmPacket[];
};

export type EbmlVint = {
  value: bigint;
  width: number;
  nextOffset: number;
  unknownSize: boolean;
};

type VintKind = "id" | "size";

type ElementHeader = {
  id: number;
  contentStart: number;
  contentEnd: number;
};

type WebmTrack = {
  trackNumber?: number;
  trackType?: number;
  codecId?: string;
  flagLacing: number;
  defaultDuration?: number;
  pixelWidth?: number;
  pixelHeight?: number;
};

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
  ReferenceBlock: 0xfb,
  Void: 0xec,
  CRC32: 0xbf,
} as const;

export function parseWebm(data: Uint8Array): WebmFile {
  const demuxer = new WebmDemuxer();
  const packets = demuxer.push(data);
  demuxer.finish();
  // finish() rejects packet-less streams, and packets imply a built header.
  return { ...demuxer.header!, packets };
}

export function readEbmlVint(data: Uint8Array, offset: number, kind: VintKind = "size"): EbmlVint {
  if (offset >= data.byteLength) {
    throw new Error(`EBML VINT at offset ${offset} is truncated`);
  }

  const first = data[offset];
  if (first === 0) {
    throw new Error(`invalid EBML VINT at offset ${offset}: first byte is zero`);
  }

  let marker = 0x80;
  let width = 1;
  while ((first & marker) === 0) {
    marker >>= 1;
    width += 1;
  }
  if (kind === "id" && width > 4) {
    throw new Error(`EBML ID at offset ${offset} is too wide: ${width} bytes`);
  }
  if (offset + width > data.byteLength) {
    throw new Error(`EBML VINT at offset ${offset} is truncated`);
  }

  let value = BigInt(kind === "id" ? first : first & ~marker);
  for (let index = 1; index < width; index += 1) {
    value = (value << 8n) | BigInt(data[offset + index]);
  }

  const maxValue = (1n << BigInt(7 * width)) - 1n;
  return {
    value,
    width,
    nextOffset: offset + width,
    unknownSize: kind === "size" && value === maxValue,
  };
}

// Incremental WebM demuxer. push() accepts stream chunks and returns the VP9
// packets completed by them; memory is bounded by the largest buffered
// element (in practice one Cluster). Elements other than EBML, Info, Tracks
// and Cluster are discarded without buffering. `header` becomes available
// once the first Cluster is fully buffered.
export class WebmDemuxer {
  private data: Uint8Array = new Uint8Array(0);
  // Parse position within `data`; everything before it is consumed.
  private cursor = 0;
  // Absolute stream offset of data[0].
  private streamOffset = 0;
  // Bytes of a skipped element still to discard, and its id for errors.
  private skipRemaining = 0;
  private skipId = 0;
  private inSegment = false;
  // Absolute end of the current Segment content; Infinity when unknown-size.
  private segmentEnd = Infinity;
  private sawSegment = false;
  private timestampScale = 1_000_000;
  private readonly tracks: WebmTrack[] = [];
  private headerValue: WebmHeader | undefined;
  private packetCount = 0;

  get header(): WebmHeader | undefined {
    return this.headerValue;
  }

  push(chunk: Uint8Array): WebmPacket[] {
    this.append(chunk);
    const packets: WebmPacket[] = [];
    this.advance(packets, false);
    return packets;
  }

  finish(): void {
    // In final mode a partial element is an error instead of a wait.
    this.advance([], true);
    if (!this.sawSegment) {
      throw new Error("WebM Segment not found");
    }
    if (this.headerValue === undefined) {
      // No Cluster was seen; run the validation a Cluster would have triggered.
      this.buildHeader();
    }
    if (this.packetCount === 0) {
      throw new Error("WebM contains no VP9 packets");
    }
  }

  private append(chunk: Uint8Array): void {
    if (this.cursor === this.data.byteLength) {
      this.streamOffset += this.data.byteLength;
      this.data = chunk;
      this.cursor = 0;
      return;
    }
    const tail = this.data.subarray(this.cursor);
    const next = new Uint8Array(tail.byteLength + chunk.byteLength);
    next.set(tail);
    next.set(chunk, tail.byteLength);
    this.streamOffset += this.cursor;
    this.data = next;
    this.cursor = 0;
  }

  private absCursor(): number {
    return this.streamOffset + this.cursor;
  }

  private advance(out: WebmPacket[], final: boolean): void {
    while (true) {
      if (this.skipRemaining > 0) {
        const take = Math.min(this.data.byteLength - this.cursor, this.skipRemaining);
        this.cursor += take;
        this.skipRemaining -= take;
        if (this.skipRemaining > 0) {
          if (final) {
            throw new Error(`element ${hexId(this.skipId)} size exceeds parent`);
          }
          return;
        }
      }
      if (this.inSegment && this.absCursor() >= this.segmentEnd) {
        this.inSegment = false;
      }
      if (this.cursor >= this.data.byteLength) {
        return;
      }

      const idVint = this.tryVint(this.cursor, "id", final);
      if (idVint === undefined) {
        return;
      }
      if (this.inSegment && this.streamOffset + idVint.nextOffset > this.segmentEnd) {
        throw new Error(`element ID at offset ${this.cursor} exceeds parent`);
      }
      const sizeVint = this.tryVint(idVint.nextOffset, "size", final);
      if (sizeVint === undefined) {
        return;
      }
      const id = toSafeNumber(idVint.value, "EBML ID");
      if (this.inSegment && this.streamOffset + sizeVint.nextOffset > this.segmentEnd) {
        throw new Error(`element ${hexId(id)} size exceeds parent`);
      }

      const topLevelSegment = id === ID.Segment && !this.inSegment;
      if (sizeVint.unknownSize && !topLevelSegment) {
        throw new Error(`element ${hexId(id)} has unsupported unknown size`);
      }
      const contentStart = sizeVint.nextOffset;

      if (topLevelSegment) {
        this.sawSegment = true;
        this.inSegment = true;
        this.segmentEnd = sizeVint.unknownSize
          ? Infinity
          : checkedAdd(
              this.streamOffset + contentStart,
              toSafeNumber(sizeVint.value, "element size"),
            );
        this.cursor = contentStart;
        continue;
      }

      const contentSize = toSafeNumber(sizeVint.value, "element size");
      if (this.inSegment && this.streamOffset + contentStart + contentSize > this.segmentEnd) {
        throw new Error(`element ${hexId(id)} size exceeds parent`);
      }

      const buffered = this.inSegment
        ? id === ID.Info || id === ID.Tracks || id === ID.Cluster
        : id === ID.EBML;
      if (!buffered) {
        this.cursor = contentStart;
        this.skipId = id;
        this.skipRemaining = contentSize;
        continue;
      }

      const contentEnd = checkedAdd(contentStart, contentSize);
      if (contentEnd > this.data.byteLength) {
        if (final) {
          throw new Error(`element ${hexId(id)} size exceeds parent`);
        }
        return;
      }
      switch (id) {
        case ID.EBML:
          this.parseEbmlHeader(contentStart, contentEnd);
          break;
        case ID.Info:
          this.parseInfo(contentStart, contentEnd);
          break;
        case ID.Tracks:
          this.parseTracks(contentStart, contentEnd);
          break;
        case ID.Cluster:
          if (this.headerValue === undefined) {
            this.buildHeader();
          }
          this.parseCluster(contentStart, contentEnd, out);
          break;
      }
      this.cursor = contentEnd;
    }
  }

  // Reads a VINT at `offset`, or returns undefined when the window is too
  // short to contain it. In final mode short reads throw instead.
  private tryVint(offset: number, kind: VintKind, final: boolean): EbmlVint | undefined {
    if (!final) {
      if (offset >= this.data.byteLength) {
        return undefined;
      }
      const first = this.data[offset];
      if (first !== 0) {
        let marker = 0x80;
        let width = 1;
        while ((first & marker) === 0) {
          marker >>= 1;
          width += 1;
        }
        const invalidWideId = kind === "id" && width > 4;
        if (!invalidWideId && offset + width > this.data.byteLength) {
          return undefined;
        }
      }
    }
    return readEbmlVint(this.data, offset, kind);
  }

  private buildHeader(): void {
    const track = this.requireSelectedTrack();
    if (track.trackNumber === undefined || track.trackNumber <= 0) {
      throw new Error("VP9 track is missing TrackNumber");
    }
    if (track.pixelWidth === undefined) {
      throw new Error("VP9 track is missing Video/PixelWidth");
    }
    if (track.pixelHeight === undefined) {
      throw new Error("VP9 track is missing Video/PixelHeight");
    }
    if (track.pixelWidth <= 0 || track.pixelHeight <= 0) {
      throw new Error(`WebM dimensions must be non-zero: ${track.pixelWidth}x${track.pixelHeight}`);
    }
    if (this.timestampScale <= 0) {
      throw new Error(`WebM TimestampScale must be non-zero: ${this.timestampScale}`);
    }
    this.headerValue = {
      codecId: "V_VP9",
      width: track.pixelWidth,
      height: track.pixelHeight,
      timestampScale: this.timestampScale,
      trackNumber: track.trackNumber,
      flagLacing: track.flagLacing,
      defaultDuration: track.defaultDuration,
    };
  }

  private parseEbmlHeader(start: number, end: number): void {
    let offset = start;
    while (offset < end) {
      const element = this.readElement(offset, end);
      switch (element.id) {
        case ID.DocType: {
          const docType = readAscii(this.data, element.contentStart, element.contentEnd);
          if (docType !== "webm") {
            throw new Error(`unsupported EBML DocType: ${docType}`);
          }
          break;
        }
        default:
          break;
      }
      offset = element.contentEnd;
    }
  }

  private parseInfo(start: number, end: number): void {
    let offset = start;
    while (offset < end) {
      const element = this.readElement(offset, end);
      switch (element.id) {
        case ID.TimestampScale:
          this.timestampScale = readUint(this.data, element.contentStart, element.contentEnd, "TimestampScale");
          break;
        default:
          break;
      }
      offset = element.contentEnd;
    }
  }

  private parseTracks(start: number, end: number): void {
    let offset = start;
    while (offset < end) {
      const element = this.readElement(offset, end);
      switch (element.id) {
        case ID.TrackEntry:
          this.tracks.push(this.parseTrackEntry(element.contentStart, element.contentEnd));
          break;
        default:
          break;
      }
      offset = element.contentEnd;
    }
  }

  private parseTrackEntry(start: number, end: number): WebmTrack {
    const track: WebmTrack = { flagLacing: 1 };
    let offset = start;
    while (offset < end) {
      const element = this.readElement(offset, end);
      switch (element.id) {
        case ID.TrackNumber:
          track.trackNumber = readUint(this.data, element.contentStart, element.contentEnd, "TrackNumber");
          break;
        case ID.TrackType:
          track.trackType = readUint(this.data, element.contentStart, element.contentEnd, "TrackType");
          break;
        case ID.CodecID:
          track.codecId = readAscii(this.data, element.contentStart, element.contentEnd);
          break;
        case ID.FlagLacing:
          track.flagLacing = readUint(this.data, element.contentStart, element.contentEnd, "FlagLacing");
          break;
        case ID.DefaultDuration:
          track.defaultDuration = readUint(this.data, element.contentStart, element.contentEnd, "DefaultDuration");
          break;
        case ID.Video:
          this.parseVideo(element.contentStart, element.contentEnd, track);
          break;
        default:
          break;
      }
      offset = element.contentEnd;
    }
    return track;
  }

  private parseVideo(start: number, end: number, track: WebmTrack): void {
    let offset = start;
    while (offset < end) {
      const element = this.readElement(offset, end);
      switch (element.id) {
        case ID.PixelWidth:
          track.pixelWidth = readUint(this.data, element.contentStart, element.contentEnd, "PixelWidth");
          break;
        case ID.PixelHeight:
          track.pixelHeight = readUint(this.data, element.contentStart, element.contentEnd, "PixelHeight");
          break;
        default:
          break;
      }
      offset = element.contentEnd;
    }
  }

  private parseCluster(start: number, end: number, out: WebmPacket[]): void {
    // buildHeader() ran before the first cluster, so the track is validated.
    const trackNumber = this.headerValue!.trackNumber;

    let clusterTimestamp = 0n;
    let offset = start;
    while (offset < end) {
      const element = this.readElement(offset, end);
      if (element.id === ID.Timestamp) {
        clusterTimestamp = BigInt(
          readUint(this.data, element.contentStart, element.contentEnd, "Cluster Timestamp"),
        );
      }
      offset = element.contentEnd;
    }

    offset = start;
    while (offset < end) {
      const element = this.readElement(offset, end);
      switch (element.id) {
        case ID.SimpleBlock:
          this.parseBlock(element.contentStart, element.contentEnd, clusterTimestamp, trackNumber, out);
          break;
        case ID.BlockGroup:
          this.parseBlockGroup(element.contentStart, element.contentEnd, clusterTimestamp, trackNumber, out);
          break;
        default:
          break;
      }
      offset = element.contentEnd;
    }
  }

  private parseBlockGroup(
    start: number,
    end: number,
    clusterTimestamp: bigint,
    selectedTrackNumber: number,
    out: WebmPacket[],
  ): void {
    const blocks: ElementHeader[] = [];
    let hasReferenceBlock = false;
    let offset = start;
    while (offset < end) {
      const element = this.readElement(offset, end);
      switch (element.id) {
        case ID.Block:
          blocks.push(element);
          break;
        case ID.ReferenceBlock:
          hasReferenceBlock = true;
          break;
        default:
          break;
      }
      offset = element.contentEnd;
    }

    for (const block of blocks) {
      this.parseBlock(block.contentStart, block.contentEnd, clusterTimestamp, selectedTrackNumber, out, {
        keyframe: !hasReferenceBlock,
      });
    }
  }

  private parseBlock(
    start: number,
    end: number,
    clusterTimestamp: bigint,
    selectedTrackNumber: number,
    out: WebmPacket[],
    options: { keyframe?: boolean } = {},
  ): void {
    const trackNumberVint = readEbmlVint(this.data, start);
    if (trackNumberVint.nextOffset + 3 > end) {
      throw new Error(`block at offset ${start} header is truncated`);
    }

    const trackNumber = toSafeNumber(trackNumberVint.value, "block TrackNumber");
    const relativeTimestamp = readI16(this.data, trackNumberVint.nextOffset);
    const flagsOffset = trackNumberVint.nextOffset + 2;
    const flags = this.data[flagsOffset];
    const payloadStart = flagsOffset + 1;

    if (trackNumber !== selectedTrackNumber) {
      return;
    }
    if ((flags & 0x06) !== 0) {
      throw new Error(`laced VP9 block on track ${selectedTrackNumber} is not supported`);
    }

    out.push({
      index: this.packetCount,
      timestamp: clusterTimestamp + BigInt(relativeTimestamp),
      keyframe: options.keyframe ?? (flags & 0x80) !== 0,
      visible: (flags & 0x08) === 0,
      // Copy out of the demux window so consumed input can be released.
      payload: this.data.slice(payloadStart, end),
    });
    this.packetCount += 1;
  }

  private requireSelectedTrack(): WebmTrack {
    const selected = this.tracks.filter((track) => track.trackType === 1 && track.codecId === "V_VP9");
    if (selected.length === 0) {
      throw new Error("WebM has no VP9 video track");
    }
    if (selected.length > 1) {
      throw new Error("WebM has multiple VP9 video tracks");
    }
    return selected[0];
  }

  private readElement(offset: number, parentEnd: number): ElementHeader {
    return readElementHeader(this.data, offset, parentEnd);
  }
}

function readElementHeader(data: Uint8Array, offset: number, parentEnd: number): ElementHeader {
  const id = readEbmlVint(data, offset, "id");
  if (id.nextOffset > parentEnd) {
    throw new Error(`element ID at offset ${offset} exceeds parent`);
  }

  const size = readEbmlVint(data, id.nextOffset);
  if (size.nextOffset > parentEnd) {
    throw new Error(`element ${hexId(toSafeNumber(id.value, "EBML ID"))} size exceeds parent`);
  }

  const numericId = toSafeNumber(id.value, "EBML ID");
  if (size.unknownSize) {
    throw new Error(`element ${hexId(numericId)} has unsupported unknown size`);
  }
  const contentStart = size.nextOffset;
  const contentEnd = checkedAdd(contentStart, toSafeNumber(size.value, "element size"));
  if (contentEnd > parentEnd) {
    throw new Error(`element ${hexId(numericId)} size exceeds parent`);
  }

  return {
    id: numericId,
    contentStart,
    contentEnd,
  };
}

function readUint(data: Uint8Array, start: number, end: number, name: string): number {
  const length = end - start;
  if (length > 8) {
    throw new Error(`${name} integer is too wide: ${length} bytes`);
  }

  let value = 0n;
  for (let offset = start; offset < end; offset += 1) {
    value = (value << 8n) | BigInt(data[offset]);
  }
  return toSafeNumber(value, name);
}

function readI16(data: Uint8Array, offset: number): number {
  let value = (data[offset] << 8) | data[offset + 1];
  if ((value & 0x8000) !== 0) {
    value -= 0x1_0000;
  }
  return value;
}

function readAscii(data: Uint8Array, start: number, end: number): string {
  let text = "";
  for (let offset = start; offset < end; offset += 1) {
    text += String.fromCharCode(data[offset]);
  }
  return text;
}

function toSafeNumber(value: bigint, name: string): number {
  if (value > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error(`${name} exceeds safe integer range`);
  }
  return Number(value);
}

function checkedAdd(a: number, b: number): number {
  const value = a + b;
  if (!Number.isSafeInteger(value)) {
    throw new Error("element size overflow");
  }
  return value;
}

function hexId(id: number): string {
  return `0x${id.toString(16)}`;
}
