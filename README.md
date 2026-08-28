# Worldline kernel bootstrap

Это Rust workspace для Grace Changes `C-KERNEL-PLUGIN-RUNTIME-BOOTSTRAP-20260827`
и `C-KERNEL-CAPABILITY-SECURITY-20260828`. Ядро намеренно не знает о browser
engine, LLM, UI, истории, вкладках или конкретном agent runtime.

В workspace входят:

- `worldline-kernel` — plugin contract, структурированные capability identities,
  dependency resolution, lifecycle scopes, owned effects, principals, in-memory
  grants, resource attenuation, revocation, invocation broker и append-only
  trajectory;
- `worldline-demo` — consumer, показывающий разницу между capability
  availability и authorization, включая grant/revoke и compatible provider
  replacement.

`CapabilityHandle` является broker proxy. Наличие resolved dependency или
активного provider не создаёт grant: вызов проходит только при наличии
подходящего active grant. Provider получает `InvocationContext` и может явно
выбрать delegated authority либо собственную authority; delegated invocation
не имеет automatic self-authority fallback.

Авторизация выполняется в момент допуска invocation: broker проверяет grant до
передачи вызова provider. Поэтому отзыв grant не отменяет уже допущенный
in-flight вызов; он блокирует последующие admissions. Вложенные вызовы также
ограничены максимальной глубиной, а `ProviderSelf` доступен только текущему
provider runtime и использует только его собственные grants.

## Проверка

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo run -p worldline-demo
```

Toolchain проекта закреплён на последнем доступном стабильном Rust `1.98.0`,
edition — `2024`.
