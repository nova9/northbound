# AGENTS.md — TrueNorth Lightweight Wallet

## Project

Build a self-custodial [TrueNorth ](https://github.com/truenorth-project/truenorth/)lightweight wallet using Rust and Dioxus, alongside a Rust/Axum API backed by `truenorthd`.

Develop the wallet and server together, one small, working feature at a time.

## Core Philosophy

**Write code for humans first, computers second.**

Code should read almost like English. A developer unfamiliar with the project should understand what happens by reading the code from top to bottom.

Prefer boring, straightforward code over clever abstractions.

**Optimize for the reader, not the person writing the code.**

## Code Style

- Use descriptive names for variables, functions, and types.
- Keep files small and focused. Aim for under 200 lines where practical, but do not enforce an arbitrary limit.
- Keep functions short, with one clear responsibility.
- Prefer explicit steps over compact expressions.
- Use early returns instead of deeply nested conditions.
- Keep control flow easy to follow.
- Avoid unnecessary traits, generics, macros, and abstractions.
- Prefer simple loops over complicated iterator chains.
- Avoid excessive cloning, but prioritize clarity over premature optimization.
- Follow standard Rust conventions and use `rustfmt`.

### Example: Prefer readable code

Avoid compact code that requires mentally unpacking several operations:

```rust
let total = utxos.iter()
    .filter(|u| u.confirmed)
    .map(|u| u.value)
    .sum::<u64>();
```

Prefer straightforward, sequential code:

```rust
let mut balance = 0;

for utxo in &utxos {
    if utxo.confirmed {
        balance += utxo.value;
    }
}
```

Both are valid Rust. Prefer the second style when it makes the logic easier to follow.

This is a preference, not a prohibition on iterators. Use an iterator when it genuinely makes the code clearer.

## No Hidden Logic

Important operations should be obvious from reading the calling code.

- Do not hide network requests, database writes, or filesystem operations behind innocent-looking function names.
- Make state changes and data transformations explicit.
- Prefer explicit arguments over global state or implicit dependencies.
- Avoid surprising side effects.
- Avoid magic numbers and unexplained defaults.
- Use descriptive names for functions that perform I/O.
- Keep the sequence of operations visible.

For example, prefer:

```rust
let utxos = fetch_utxos(&client, &address).await?;
let balance = calculate_balance(&utxos);
update_balance_display(balance);
```

Over a function such as `refresh_wallet()` that silently fetches data, updates application state, and performs unrelated work.

## Rust Guidelines

- Use structs and enums when they make the domain easier to understand.
- Prefer simple functions over complicated object-oriented patterns.
- Use `Result` and `?` for ordinary error propagation.
- Use meaningful error messages.
- Avoid `unwrap()` and `expect()` in production paths.
- Avoid `unsafe` unless absolutely necessary and justified.
- Use `async` only when needed.
- Avoid unnecessary `Arc`, `Mutex`, and cloning.
- Explain non-obvious lifetime or borrowing decisions in comments.
- Prefer straightforward `match` expressions when handling different cases.
- Use standard library functionality before adding dependencies.

Don't force Rust code to imitate Python. Use idiomatic Rust when it improves correctness and clarity.

## File Organization

Start with a simple workspace:

```text
wallet/
server/
shared/
```

Organize modules by responsibility, not artificial architectural layers.

### File splitting rules

- Aim for files under 200 lines when practical.
- A file should have one clear purpose.
- Split files when they contain unrelated responsibilities or become difficult to navigate.
- Do not create a separate file for every small function.
- Do not create unnecessary layers of modules.
- Avoid giant `main.rs`, `lib.rs`, and `mod.rs` files.
- Keep closely related logic together.

**Important:** Do not split code into so many tiny functions or files that understanding one operation requires jumping between ten locations.

A 250-line readable file is better than ten unnecessarily fragmented files.

## Function Design

Functions should express meaningful actions.

Good examples:

```rust
fetch_utxos()
calculate_balance()
create_transaction()
sign_transaction()
broadcast_transaction()
```

Avoid vague names:

```rust
process()
handle()
execute()
do_work()
manage_data()
```

Each function should have a clear purpose, predictable inputs, and an understandable result.

Avoid functions that silently perform several unrelated operations.

### Don't over-extract functions

Do not create helper functions for every two or three lines of code.

Extract a function when it:

- Represents a meaningful operation.
- Removes substantial duplication.
- Makes the calling code easier to understand.
- Separates an important responsibility.

Keep simple logic inline when extraction would make understanding harder.

## Wallet Rules

- Keep private keys and seed phrases on the user's device.
- Generate addresses and sign transactions locally.
- Never transmit secrets to the server.
- Separate wallet logic from Iced UI code.
- Keep application state and message handling easy to trace.
- Make network requests explicit.
- Clearly display transaction amounts, fees, and recipients before signing.
- Never silently modify transaction details.
- Support recovery and backup without requiring the original server.

## Server Rules

- Use straightforward Axum handlers.
- Keep HTTP handlers thin but understandable.
- Separate blockchain RPC operations from HTTP response formatting.
- Use consistent JSON responses and errors.
- Do not expose `truenorthd` administrative RPC publicly.
- Validate inputs and enforce resource limits.
- Do not add authentication or accounts without a demonstrated need.
- Introduce a database only when necessary.
- Keep endpoints independent of wallet-specific UI behavior.

## Development Workflow

Develop one complete feature at a time.

1. Inspect the relevant existing code.
2. Identify the simplest implementation.
3. Implement the server functionality.
4. Connect it to the wallet.
5. Test the complete feature.
6. Run formatting, compilation, and relevant tests.
7. Explain what changed and why.

Do not implement unrelated functionality or perform large refactors without a clear reason.

### Make small changes

- Prefer small, focused commits.
- Avoid changing unrelated files.
- Do not rewrite existing code merely to match personal style preferences.
- Preserve working behavior unless a change is necessary.
- Avoid introducing infrastructure for hypothetical future requirements.

## Dependencies and Abstractions

Every dependency and abstraction should justify its existence.

- Prefer the Rust standard library when sufficient.
- Avoid introducing frameworks for simple operations.
- Don't create traits just because multiple implementations might exist someday.
- Introduce traits when there is a real need for interchangeable implementations or testing.
- Avoid generic repository, service, factory, and manager layers without a concrete reason.
- Avoid wrapping existing libraries in unnecessary abstractions.

**Build for current requirements, not imagined future complexity.**

## Error Handling

- Never silently ignore errors.
- Return errors with useful context.
- Distinguish recoverable errors from fatal failures.
- Show understandable messages to wallet users.
- Avoid exposing secrets or internal infrastructure details in error responses.
- Do not replace meaningful errors with generic success or failure values.

## Comments and Documentation

- Prefer self-explanatory code over comments.
- Explain why something is necessary when the reason isn't obvious.
- Don't write comments that merely repeat the code.
- Explain important blockchain concepts and security decisions in plain English.
- Document API behavior and important assumptions.
- Keep documentation short and practical.

For blockchain-specific code, explain unfamiliar concepts such as UTXOs, witnesses, and P2QRH commitments where needed.

## Testing

- Test meaningful behavior, not implementation details.
- Keep tests small and readable.
- Prefer realistic test data.
- Use TrueNorth regtest for end-to-end blockchain operations.
- Test P2QRH behavior against the actual TrueNorth implementation.
- Cover transaction amounts, fees, signing, and malformed input.
- Avoid excessive mocking when real integration tests are practical.

## Security

Simple code must still be secure.

- Never log private keys, seeds, or other secrets.
- Never implement custom cryptographic primitives.
- Treat responses from remote servers as untrusted.
- Validate transaction details before signing.
- Verify TrueNorth-specific consensus behavior rather than assuming Bitcoin compatibility.
- Avoid floating-point arithmetic for cryptocurrency amounts.
- Use checked arithmetic where overflow could affect balances, fees, or transactions.
- Do not compromise correctness for shorter code.

## Avoid

- Overengineering
- Clever one-liners
- Premature optimization
- Unnecessary design patterns
- Generic frameworks for one use case
- Huge files or functions
- Hidden side effects
- Excessive dependencies
- Silent error handling
- Unnecessary background tasks
- Speculative features
- Excessive module fragmentation

## Decision Priority

When choosing between implementations, prioritize:

1. Correctness and security
2. Readability
3. Simplicity
4. Maintainability
5. Performance

If a more complex implementation is necessary for security or correctness, use it, but keep its interface simple and explain the complexity.

## Final Principle

**The best implementation is the simplest one that is correct, secure, and easy to understand.**

A developer should be able to follow an operation from beginning to end without reconstructing hidden behavior or navigating unnecessary layers.

When in doubt, choose the implementation that a new developer could understand most quickly.