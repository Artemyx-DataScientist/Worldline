# Worldline kernel bootstrap

Это начальный Rust workspace для Grace Change `C-KERNEL-PLUGIN-RUNTIME-BOOTSTRAP-20260827`.
Ядро намеренно не знает о browser engine, LLM, UI, истории, вкладках или агентах.

В workspace входят:

- `worldline-kernel` — plugin contract, структурированные capability identities,
  dependency resolution, lifecycle scopes, owned effects и append-only trajectory;
- `worldline-demo` — consumer, который работает с двумя независимыми providers
  одной capability без изменения consumer-кода.

## Проверка

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo run -p worldline-demo
```

Toolchain проекта закреплён на последнем доступном стабильном Rust `1.98.0`,
edition — `2024`.

