# vip9r

vip9r is an LLM-optimized software VP9 decoder targeting WebAssembly.

## Development

For any tasks in this repo, read `docs/requirements.md` first to understand your
roles and responsibilities.

## Performance testing

- Use release build for wasm module measurements, debug build is too slow.
- With multiple timing samples, the most favorable sample is more important than
  the average.

## Taste reminders

- Do not add "nice to have" features, unused generic tooling affordances, or
  backwards compatibility schemes. Everything in this project is a development
  convenience.
- UX drives and justifies implementation. If existing implementation conflicts
  with desired UX, prioritize the UX.
