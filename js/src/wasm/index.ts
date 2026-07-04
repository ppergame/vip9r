export type Plane = {
  offset: number;
  byteLength: number;
  stride: number;
};

export type NativeFrame = {
  decodedWidth: number;
  decodedHeight: number;
  renderWidth: number;
  renderHeight: number;
  y: Plane;
  u: Plane;
  v: Plane;
};

export type DecodeStep =
  | { kind: "no-output"; packetDone: boolean }
  | { kind: "output"; packetDone: boolean; frame: NativeFrame };

type Vip9rExports = {
  memory: WebAssembly.Memory;
  vip9r_result_ptr(): number;
  vip9r_init(maxWidth: number, maxHeight: number): number;
  vip9r_reserve_input(len: number): number;
  vip9r_begin_packet(len: number): number;
  vip9r_decode_next(): number;
};

const enum ResultField {
  InputPtr = 0,
  InputCapacity = 1,
  PacketLen = 2,
  CodedFrameCount = 3,
  CodedFrameIndex = 4,
  HasOutput = 5,
  PacketDone = 6,
  DecodedWidth = 7,
  DecodedHeight = 8,
  RenderWidth = 9,
  RenderHeight = 10,
  YPtr = 11,
  YLen = 12,
  YStride = 13,
  UPtr = 14,
  ULen = 15,
  UStride = 16,
  VPtr = 17,
  VLen = 18,
  VStride = 19,
  Count = 20,
}

export class Vip9rWasmError extends Error {
  constructor(
    readonly operation: string,
    readonly code: number,
  ) {
    super(`${operation}: ${statusName(code)} (${code})`);
    this.name = "Vip9rWasmError";
  }
}

export class Vp9Decoder {
  private readonly exports: Vip9rExports;
  private readonly resultPtr: number;
  private result: Uint32Array;
  private bytes: Uint8Array;

  constructor(instance: WebAssembly.Instance, maxWidth: number, maxHeight: number) {
    this.exports = asVip9rExports(instance.exports);
    this.resultPtr = this.exports.vip9r_result_ptr();
    this.bytes = new Uint8Array(this.exports.memory.buffer);
    this.result = new Uint32Array(
      this.exports.memory.buffer,
      this.resultPtr,
      ResultField.Count,
    );
    checkStatus("vip9r_init", this.exports.vip9r_init(maxWidth, maxHeight));
    this.refreshViews();
  }

  beginPacket(packet: Uint8Array): void {
    checkStatus("vip9r_reserve_input", this.exports.vip9r_reserve_input(packet.byteLength));
    this.refreshViews();

    const inputPtr = this.result[ResultField.InputPtr];
    const inputCapacity = this.result[ResultField.InputCapacity];
    if (packet.byteLength > inputCapacity) {
      throw new Error(`reserved input too small: ${inputCapacity} < ${packet.byteLength}`);
    }
    this.bytes.set(packet, inputPtr);

    checkStatus("vip9r_begin_packet", this.exports.vip9r_begin_packet(packet.byteLength));
  }

  decodeNext(): DecodeStep {
    checkStatus("vip9r_decode_next", this.exports.vip9r_decode_next());
    this.refreshViews();

    const packetDone = this.result[ResultField.PacketDone] !== 0;
    if (this.result[ResultField.HasOutput] === 0) {
      return { kind: "no-output", packetDone };
    }

    return {
      kind: "output",
      packetDone,
      frame: {
        decodedWidth: this.result[ResultField.DecodedWidth],
        decodedHeight: this.result[ResultField.DecodedHeight],
        renderWidth: this.result[ResultField.RenderWidth],
        renderHeight: this.result[ResultField.RenderHeight],
        y: this.plane(ResultField.YPtr, ResultField.YLen, ResultField.YStride),
        u: this.plane(ResultField.UPtr, ResultField.ULen, ResultField.UStride),
        v: this.plane(ResultField.VPtr, ResultField.VLen, ResultField.VStride),
      },
    };
  }

  planeBytes(plane: Plane): Uint8Array {
    return new Uint8Array(this.exports.memory.buffer, plane.offset, plane.byteLength);
  }

  memoryByteLength(): number {
    return this.exports.memory.buffer.byteLength;
  }

  private refreshViews(): void {
    this.bytes = new Uint8Array(this.exports.memory.buffer);
    this.result = new Uint32Array(
      this.exports.memory.buffer,
      this.resultPtr,
      ResultField.Count,
    );
  }

  private plane(ptr: ResultField, len: ResultField, stride: ResultField): Plane {
    return {
      offset: this.result[ptr],
      byteLength: this.result[len],
      stride: this.result[stride],
    };
  }
}

function asVip9rExports(exports: WebAssembly.Exports): Vip9rExports {
  const required = [
    "memory",
    "vip9r_result_ptr",
    "vip9r_init",
    "vip9r_reserve_input",
    "vip9r_begin_packet",
    "vip9r_decode_next",
  ] as const;
  for (const name of required) {
    if (!(name in exports)) {
      throw new Error(`missing wasm export: ${name}`);
    }
  }
  return exports as Vip9rExports;
}

function checkStatus(operation: string, code: number): void {
  if (code < 0) {
    throw new Vip9rWasmError(operation, code);
  }
}

function statusName(code: number): string {
  switch (code) {
    case -1:
      return "invalid config";
    case -3:
      return "resource limit";
    case -4:
      return "unsupported profile";
    case -5:
      return "unsupported bit depth";
    case -6:
      return "invalid bitstream";
    case -8:
      return "unimplemented";
    case -9:
      return "invalid state";
    default:
      return "wasm error";
  }
}
