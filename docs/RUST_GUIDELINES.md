# Rust Coding Guidelines

This project follows the **[Microsoft Pragmatic Rust Guidelines][msft]**. They
codify idiomatic Rust for libraries and applications — error handling, API
design, `unsafe` discipline, static verification, lint posture — and we have
adopted them wholesale with a small set of documented exceptions listed at the
bottom of this document.

[msft]: https://microsoft.github.io/rust-guidelines/

Read the upstream document in full when in doubt. This page maps each guideline
to how it is enforced **in this repository** — either by a machine check that
runs in CI, or by a human rule applied at review time.

## Machine-enforced

These are enforced by lints, formatters, and CI gates. A pull request cannot
merge while any of them fails.

| Upstream rule | Tool / lint | Where configured | How it fails |
| --- | --- | --- | --- |
| **M-STATIC-VERIFICATION** (compile-time checks) | `cargo fmt`, `cargo clippy -- -D warnings`, `cargo doc -D warnings` | [Cargo.toml `[workspace.lints]`](../Cargo.toml), [.github/workflows/ci.yml](../.github/workflows/ci.yml) | CI `fmt` / `clippy` / `doc` jobs fail on any warning |
| **M-UNSAFE** (every `unsafe` block justified) | `clippy::undocumented_unsafe_blocks = "deny"` | Cargo.toml | Build fails if any `unsafe` block or `unsafe impl` lacks a `// SAFETY: …` comment |
| **M-PANIC-IS-STOP** / **M-PANIC-ON-BUG** | `clippy::correctness` + `clippy::suspicious` = `"deny"` | Cargo.toml | Many panic-adjacent bugs caught at lint time; the remainder is human-reviewed |
| **M-LINT-OVERRIDE-EXPECT** (`#[expect]` over `#[allow]`) | `clippy::allow_attributes_without_reason = "warn"`, MSRV 1.81 lets us require `#[expect(..., reason = "…")]` | Cargo.toml | CI `clippy` job warns on bare `#[allow]`; the convention is enforced in code review |
| **M-PUBLIC-DEBUG** (all public types `: Debug`) | `missing_debug_implementations = "warn"` | Cargo.toml | CI `clippy` fails via `-D warnings` |
| **M-CANONICAL-DOCS** (summary + sections on `pub` items) | `missing_docs = "warn"` (published crates), `cargo doc -D warnings` | Cargo.toml, workflow `doc` job | Missing rustdoc on any published crate item fails the `doc` job |
| **Integer cast discipline** (ban on narrowing `as`; see [Integer casts](#error-handling) below) | `clippy::cast_possible_truncation`, `cast_sign_loss`, `cast_possible_wrap` all `"deny"`; `cast_lossless`, `cast_precision_loss` `"warn"` | Cargo.toml | Build fails on any narrowing integer `as` cast |
| **Supply-chain: licenses** (M-OOBE adjacent) | `cargo deny check` | [deny.toml](../deny.toml), CI `deny` job | Fails on any dependency with a non-permissive license, or any unknown registry/git source |
| **Supply-chain: advisories** | `cargo audit --deny warnings` | CI `audit` job | Fails on any unfixed RustSec advisory for a crate in the lockfile |
| **M-OOBE** (builds on Tier 1 platforms without extras) | MSRV check, workspace build | CI `test` job across linux/macos/windows | Fails if a direct or transitive dep requires a newer toolchain |

Every `#[expect(lint_name, reason = "…")]` in the tree is a waiver of one of
the above. Adding a new one is a conscious opt-out; review comments should
push back on anything that does not carry a convincing reason.

## Human-review

The rules below cannot (yet) be mechanically checked. They apply at code
review; point to them when requesting changes.

### API design

- **M-CONCISE-NAMES.** Avoid weasel words (`Service`, `Manager`, `Factory`,
  `Helper`). Prefer names that describe what the type *is* or *does*. One
  legitimate exception in this repo: `ConnectionManager` in
  [hyperdb-api/src/pool.rs](../hyperdb-api/src/pool.rs), which matches
  `deadpool::Manager` trait nomenclature.
- **M-REGULAR-FN.** Associated functions are for construction (`Type::new`,
  `Type::from_str`). Everything else is a free function or an inherent
  method; do not namespace utilities on a type for no reason.
- **M-ESSENTIAL-FN-INHERENT.** Core behavior is an inherent method; traits
  forward to it. Do not force users to `use` a trait to call a method they
  expect to exist on the type.
- **M-INIT-BUILDER.** Four or more initialization permutations → a builder.
  No `set_foo(&mut self, …)` after construction for things that could have
  been a builder.
- **M-IMPL-ASREF / M-IMPL-IO.** Public functions that take paths, strings, or
  I/O readers accept `impl AsRef<Path>` / `impl io::Read` rather than
  concrete types. Types themselves do not carry these bounds.
- **M-SIMPLE-ABSTRACTIONS.** Keep visible type-parameter nesting shallow in
  public APIs. If a signature contains more than two nested generic
  parameters, look for a helper type.
- **M-DONT-LEAK-TYPES.** Prefer `std` types in public APIs. Third-party types
  (`bytes::Bytes`, `arrow::RecordBatch`, …) are only exposed when they
  materially improve the API over an equivalent in `std`.

### Error handling

- **M-APP-ERROR** / **M-ERRORS-CANONICAL-STRUCTS.** Library crates
  (`hyperdb-api`, `hyperdb-api-salesforce`, `sea-query-hyperdb`) return canonical
  error enums with `Display`, `Error`, and a public constructor per variant.
  Application crates (`hyperdb-mcp`, examples) may use `anyhow`.
- **Integer casts** (repo-specific extension; the cast clippy lints are
  `deny`-level in this workspace, see `Cargo.toml`). Choose the right tool
  for each narrowing conversion:
  - Caller can tolerate failure → `T::try_from(x).ok()?` or `.map_err(...)?`
  - Caller-validated invariant → `T::try_from(x).expect("<reason>")`
  - Always fits by type algebra → `#[expect(clippy::cast_*, reason = "<proof>")]`
  - Bit-pattern reinterpret (encode/decode pairs) → `#[expect]` with the
    word "reinterpret" in the reason.
  Never introduce a new bare `as` cast between differently-sized integer
  types. The `cast_possible_truncation` / `cast_sign_loss` /
  `cast_possible_wrap` lints will block it; the lints exist to prompt a
  choice, not to block mechanically.

### Documentation

- **M-CANONICAL-DOCS.** Every `pub` item on a published crate has: a one-line
  summary sentence, optional extended prose, and the applicable sections
  (`# Examples`, `# Errors`, `# Panics`, `# Safety`). See
  [RUST_DOCUMENTATION_STYLE.md](RUST_DOCUMENTATION_STYLE.md) for the
  repo-specific conventions.
- **M-FIRST-DOC-SENTENCE.** The first sentence is under 15 words and on a
  single line; docs.rs renders it as the type's search-result snippet.
- **M-DOCUMENTED-MAGIC.** Magic numbers (`MICROS_PER_DAY`,
  `JULIAN_DAY_EPOCH`, the ~2 GiB wire-message ceiling) are `const`s with a
  comment that explains the choice, not inline literals.

### Logging

- **M-LOG-STRUCTURED.** `tracing` event macros with named fields, no
  `format!`-built strings. Sensitive data (passwords, tokens, OAuth refresh
  tokens) is redacted at the emit site, not in a formatter.

## Exceptions

Lint-level relaxations live in [the root Cargo.toml](../Cargo.toml) under
`[workspace.lints.clippy]`. Microsoft's guidelines explicitly permit
per-lint opt-outs where a pedantic lint proves noisy — the blanket policy
stays at `warn`, and this table enumerates the individual exceptions.

| Lint | Level | Reason |
| --- | --- | --- |
| `clippy::module_name_repetitions` | `allow` | Existing crate/type naming is intentional (e.g. `hyperdb_api_core::types::Numeric` → `hyperdb_api_core::types::NumericError`) and churning it does not improve readability. |
| `clippy::too_many_lines` | `allow` | Style preference, not worth the churn. Prefer reviewer judgment. |
| `clippy::doc_markdown` | `allow` | Cosmetic: backticking every type name in rustdoc is churn with low reader benefit. |
| `clippy::must_use_candidate` | `allow` | API-judgment call per method — promoting to `warn` post-1.0 with a focused API audit. |
| `clippy::unreadable_literal` | `allow` | Cosmetic: digit separators on wire-format constants (`MAX_JULIAN_DAY = 5373484`) reduce grep-ability. |
| `clippy::items_after_statements` | `allow` | Stylistic: benchmarks and tests use local helpers intermixed with setup logic. |
| `clippy::match_same_arms` | `allow` | Consolidating identical arms can hide semantic grouping (e.g. SQL-type size tables). |
| `clippy::missing_errors_doc` | `warn` (to be promoted) | Currently a large number of sites; promote to `deny` once the post-1.0 docs pass closes them out. |
| `clippy::missing_panics_doc` | `warn` (to be promoted) | Same as above. |

Any **source-level** waiver is expressed as
`#[expect(lint_name, reason = "<specific reason>")]`. That attribute both
silences the lint and warns if the lint *would* no longer fire — forcing a
periodic garbage-collection of stale waivers. Never use `#[allow(...)]`
without a `reason`; `clippy::allow_attributes_without_reason` will flag it.

## Further reading

- [AGENTS.md](../AGENTS.md) — repo-wide coding conventions and the
  `HYPERD_PATH` build quirk.
- [CONTRIBUTING.md](../CONTRIBUTING.md) — governance, commit conventions,
  release process.
- [RUST_DOCUMENTATION_STYLE.md](RUST_DOCUMENTATION_STYLE.md) — doc-style
  rules for rustdoc, READMEs, and `docs/`.
- [DEVELOPMENT.md](../DEVELOPMENT.md) — workspace architecture and build.
