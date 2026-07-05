// Test-only builders for synthesizing WebM byte streams.

export const ID = {
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
  Cues: 0x1c53bb6b,
} as const;

export function webmFile(
  options: {
    tracks?: Uint8Array[];
    segmentExtras?: Uint8Array[];
    clusters?: Uint8Array[];
    segmentDeclaredSizeExtra?: number;
  } = {},
): Uint8Array {
  const segmentContent = segmentContentBytes(options);
  return concat(
    element(ID.EBML, stringElement(ID.DocType, "webm")),
    elementWithSize(
      ID.Segment,
      segmentContent.byteLength + (options.segmentDeclaredSizeExtra ?? 0),
      segmentContent,
    ),
  );
}

export function segmentContentBytes(
  options: {
    tracks?: Uint8Array[];
    segmentExtras?: Uint8Array[];
    clusters?: Uint8Array[];
  } = {},
): Uint8Array {
  const tracks = options.tracks ?? [vp9Track(1, { width: 320, height: 240 })];
  const clusters = options.clusters ?? [
    cluster(0, [simpleBlock(1, 0, 0, [1])]),
  ];
  return concat(
    element(ID.Info, uintElement(ID.TimestampScale, 1_000_000)),
    element(ID.Tracks, concat(...tracks)),
    ...(options.segmentExtras ?? []),
    ...clusters,
  );
}

export function vp9Track(
  trackNumber: number,
  dimensions: { width: number; height: number },
): Uint8Array {
  return trackEntry([
    uintElement(ID.TrackNumber, trackNumber),
    uintElement(ID.TrackType, 1),
    stringElement(ID.CodecID, "V_VP9"),
    uintElement(ID.FlagLacing, 0),
    uintElement(ID.DefaultDuration, 33_333_333),
    element(
      ID.Video,
      concat(
        uintElement(ID.PixelWidth, dimensions.width),
        uintElement(ID.PixelHeight, dimensions.height),
      ),
    ),
  ]);
}

export function audioTrack(trackNumber: number): Uint8Array {
  return trackEntry([
    uintElement(ID.TrackNumber, trackNumber),
    uintElement(ID.TrackType, 2),
    stringElement(ID.CodecID, "A_OPUS"),
  ]);
}

export function trackEntry(fields: Uint8Array[]): Uint8Array {
  return element(ID.TrackEntry, concat(...fields));
}

export function cluster(timestamp: number, blocks: Uint8Array[]): Uint8Array {
  return element(
    ID.Cluster,
    concat(uintElement(ID.Timestamp, timestamp), ...blocks),
  );
}

export function simpleBlock(
  trackNumber: number,
  relativeTimestamp: number,
  flags: number,
  payload: number[],
): Uint8Array {
  return element(
    ID.SimpleBlock,
    blockContent(trackNumber, relativeTimestamp, flags, payload),
  );
}

export function block(
  trackNumber: number,
  relativeTimestamp: number,
  flags: number,
  payload: number[],
): Uint8Array {
  return element(
    ID.Block,
    blockContent(trackNumber, relativeTimestamp, flags, payload),
  );
}

export function blockGroup(blockElement: Uint8Array): Uint8Array {
  return element(ID.BlockGroup, blockElement);
}

export function blockContent(
  trackNumber: number,
  relativeTimestamp: number,
  flags: number,
  payload: number[],
): Uint8Array {
  if (trackNumber < 1 || trackNumber > 126) {
    throw new Error(
      `test fixture track number is out of range: ${trackNumber}`,
    );
  }
  const timestamp =
    relativeTimestamp < 0 ? 0x1_0000 + relativeTimestamp : relativeTimestamp;
  return new Uint8Array([
    0x80 | trackNumber,
    (timestamp >>> 8) & 0xff,
    timestamp & 0xff,
    flags,
    ...payload,
  ]);
}

export function element(id: number, content: Uint8Array): Uint8Array {
  return elementWithSize(id, content.byteLength, content);
}

export function elementWithSize(
  id: number,
  declaredSize: number,
  content: Uint8Array,
): Uint8Array {
  return concat(
    new Uint8Array(idBytes(id)),
    new Uint8Array(sizeVint(declaredSize)),
    content,
  );
}

export function unknownSizeElement(
  id: number,
  content: Uint8Array,
): Uint8Array {
  return concat(new Uint8Array(idBytes(id)), new Uint8Array([0xff]), content);
}

export function uintElement(id: number, value: number): Uint8Array {
  return element(id, new Uint8Array(uintBytes(value)));
}

export function intElement(id: number, value: number): Uint8Array {
  return element(id, new Uint8Array(intBytes(value)));
}

export function stringElement(id: number, value: string): Uint8Array {
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
    return [
      0x10 | ((size >>> 24) & 0x0f),
      (size >>> 16) & 0xff,
      (size >>> 8) & 0xff,
      size & 0xff,
    ];
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

function intBytes(value: number): number[] {
  if (value < -128 || value > 127) {
    throw new Error(`test fixture signed integer is out of range: ${value}`);
  }
  return [value < 0 ? 0x100 + value : value];
}

export function concat(...parts: Uint8Array[]): Uint8Array {
  const length = parts.reduce((sum, part) => sum + part.byteLength, 0);
  const out = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.byteLength;
  }
  return out;
}
