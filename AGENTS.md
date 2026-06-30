# vip9r

vip9r is an LLM-optimized software VP9 decoder targeting WebAssembly.

## Development

For any tasks in this repo, read `docs/requirements.md` first to understand your
roles and responsibilities.

## Performance testing

- Use release build for wasm module measurements, debug build is too slow.
- With multiple samples, the most favorable sample is more important than the average.