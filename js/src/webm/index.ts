export type WebmPacket = {
  index: number;
  timestamp: bigint;
  payload: Uint8Array;
};

export type WebmFile = {
  codecId: "V_VP9";
  width: number;
  height: number;
  timestampScale: number;
  trackNumber: number;
  flagLacing: number;
  defaultDuration?: number;
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
  unknownSize: boolean;
  truncatedContent: boolean;
};

type ReadElementOptions = {
  allowUnknownSize?: boolean;
  allowTruncatedContent?: boolean;
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
  Void: 0xec,
  CRC32: 0xbf,
} as const;

export function parseWebm(data: Uint8Array): WebmFile {
  return new WebmParser(data).parse();
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

class WebmParser {
  private readonly tracks: WebmTrack[] = [];
  private readonly packets: WebmPacket[] = [];
  private timestampScale = 1_000_000;
  private sawSegment = false;

  constructor(private readonly data: Uint8Array) {}

  parse(): WebmFile {
    let offset = 0;
    while (offset < this.data.byteLength) {
      const element = this.readElement(offset, this.data.byteLength, {
        allowUnknownSize: true,
        allowTruncatedContent: true,
      });
      if (element.unknownSize && element.id !== ID.Segment) {
        throw new Error(`element ${hexId(element.id)} has unsupported unknown size`);
      }
      if (element.truncatedContent && element.id !== ID.Segment) {
        throw new Error(`element ${hexId(element.id)} size exceeds parent`);
      }

      switch (element.id) {
        case ID.EBML:
          this.parseEbmlHeader(element.contentStart, element.contentEnd);
          break;
        case ID.Segment:
          this.sawSegment = true;
          this.parseSegment(element.contentStart, element.contentEnd);
          break;
        case ID.Void:
        case ID.CRC32:
          break;
        default:
          break;
      }
      offset = element.contentEnd;
    }

    if (!this.sawSegment) {
      throw new Error("WebM Segment not found");
    }
    if (this.timestampScale <= 0) {
      throw new Error(`WebM TimestampScale must be non-zero: ${this.timestampScale}`);
    }

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
    if (this.packets.length === 0) {
      throw new Error("WebM contains no VP9 packets");
    }

    return {
      codecId: "V_VP9",
      width: track.pixelWidth,
      height: track.pixelHeight,
      timestampScale: this.timestampScale,
      trackNumber: track.trackNumber,
      flagLacing: track.flagLacing,
      defaultDuration: track.defaultDuration,
      packets: this.packets,
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

  private parseSegment(start: number, end: number): void {
    let offset = start;
    while (offset < end) {
      const element = this.readElement(offset, end);
      switch (element.id) {
        case ID.Info:
          this.parseInfo(element.contentStart, element.contentEnd);
          break;
        case ID.Tracks:
          this.parseTracks(element.contentStart, element.contentEnd);
          break;
        case ID.Cluster:
          this.parseCluster(element.contentStart, element.contentEnd);
          break;
        case ID.Void:
        case ID.CRC32:
          break;
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

  private parseCluster(start: number, end: number): void {
    const selectedTrack = this.requireSelectedTrack();
    if (selectedTrack.trackNumber === undefined || selectedTrack.trackNumber <= 0) {
      throw new Error("VP9 track is missing TrackNumber");
    }

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
          this.parseBlock(element.contentStart, element.contentEnd, clusterTimestamp, selectedTrack.trackNumber);
          break;
        case ID.BlockGroup:
          this.parseBlockGroup(element.contentStart, element.contentEnd, clusterTimestamp, selectedTrack.trackNumber);
          break;
        default:
          break;
      }
      offset = element.contentEnd;
    }
  }

  private parseBlockGroup(start: number, end: number, clusterTimestamp: bigint, selectedTrackNumber: number): void {
    let offset = start;
    while (offset < end) {
      const element = this.readElement(offset, end);
      switch (element.id) {
        case ID.Block:
          this.parseBlock(element.contentStart, element.contentEnd, clusterTimestamp, selectedTrackNumber);
          break;
        default:
          break;
      }
      offset = element.contentEnd;
    }
  }

  private parseBlock(start: number, end: number, clusterTimestamp: bigint, selectedTrackNumber: number): void {
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

    this.packets.push({
      index: this.packets.length,
      timestamp: clusterTimestamp + BigInt(relativeTimestamp),
      payload: this.data.subarray(payloadStart, end),
    });
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

  private readElement(offset: number, parentEnd: number, options: ReadElementOptions = {}): ElementHeader {
    const element = readElementHeader(this.data, offset, parentEnd, options.allowTruncatedContent ?? false);
    if (element.unknownSize && !(options.allowUnknownSize ?? false)) {
      throw new Error(`element ${hexId(element.id)} has unsupported unknown size`);
    }
    return element;
  }
}

function readElementHeader(
  data: Uint8Array,
  offset: number,
  parentEnd: number,
  allowTruncatedContent: boolean,
): ElementHeader {
  const id = readEbmlVint(data, offset, "id");
  if (id.nextOffset > parentEnd) {
    throw new Error(`element ID at offset ${offset} exceeds parent`);
  }

  const size = readEbmlVint(data, id.nextOffset);
  if (size.nextOffset > parentEnd) {
    throw new Error(`element ${hexId(toSafeNumber(id.value, "EBML ID"))} size exceeds parent`);
  }

  const numericId = toSafeNumber(id.value, "EBML ID");
  const contentStart = size.nextOffset;
  let contentEnd = size.unknownSize
    ? parentEnd
    : checkedAdd(contentStart, toSafeNumber(size.value, "element size"));
  const truncatedContent = contentEnd > parentEnd;
  if (truncatedContent && allowTruncatedContent) {
    contentEnd = parentEnd;
  }
  if (contentEnd > parentEnd) {
    throw new Error(`element ${hexId(numericId)} size exceeds parent`);
  }

  return {
    id: numericId,
    contentStart,
    contentEnd,
    unknownSize: size.unknownSize,
    truncatedContent,
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
