// ABI constants of the fixed-address shadow-stack layout baked into the wasm
// link (rust/.cargo/config.toml; mechanics in docs/design.md, Threads):
// coordinator stack 0..1 MiB, three fixed 1 MiB worker regions above it, data
// pushed past them by --global-base. Worker spawners rebind each worker
// instance's __stack_pointer to its region top before any wasm runs.
export const COORDINATOR_STACK_TOP = 0x100000;
export const WORKER_STACK_TOPS = [0x200000, 0x300000, 0x400000] as const;
export const MIN_HEAP_BASE = 0x400000;
