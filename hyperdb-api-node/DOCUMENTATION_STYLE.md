# JavaScript/TypeScript Documentation Style Guide

This file contains the JS/TS-specific style guide for developer-targeted documentation
in the `hyperdb-api-node` package.

We distinguish 4 types of documentation:
* **Source code documentation**: Document exported functions, classes, methods, and constants
  using JSDoc comments (`/** ... */`). These render in editor tooltips (VS Code, WebStorm)
  and can be extracted by documentation generators.
* **Package README**: Document the goals, installation, quick start, and API reference for
  npm users in `README.md`. This renders on npmjs.com.
* **Process and architecture documentation**: Document general processes (building, testing,
  publishing) and cross-cutting architecture in `DEVELOPMENT.md` or the top-level `docs/`
  folder.
* **Commit messages**: Should contain the *why* a change was made. Short-lived context
  belongs in the commit description, not in the code.


## General Writing Rules

Be concise and to the point.
Assume readers are fluent in JavaScript/TypeScript and familiar with Node.js.

Be precise and avoid vague statements.
Prefer specific names over vague backreferences (e.g., "`ConnectionPool`" over "the pool"
when context is ambiguous).

Write all documentation in American English.

When writing documentation, be inclusive of both AI agents and humans.


### Cross-Referencing vs. Duplication

If content applies to both Rust and JS/TS layers or describes a cross-cutting workflow,
it belongs in the top-level `docs/` folder or `DEVELOPMENT.md`. The package README should
cross-reference it rather than duplicating the content.

Information specific to the JS/TS package stays in the package's own files (README,
DEVELOPMENT.md, or source code).


## Package README

The `README.md` serves two audiences: developers browsing the repository, and users
viewing the package on npmjs.com.

It should start with a one-liner description of what the package does.

It should explain installation, quick start (with both TypeScript and JavaScript examples),
and provide a concise API reference. Show *how to use*, not *how it works internally*.

### npm Rendering Constraints

The README renders on npmjs.com, which differs from GitHub:
* **Mermaid diagrams** are supported on GitHub but **not on npmjs.com**. Use ASCII diagrams
  or fenced code blocks for npm compatibility, or accept that diagrams only render on GitHub.
* **Relative links** to other files in the repo work on GitHub but not on npm. Use absolute
  URLs for cross-references that must work on both.
* **Badges** — place CI/version/downloads badges at the top.


## Source Code Documentation (JSDoc)

### Conventions

Use `/** ... */` (JSDoc) for all exported functions, classes, methods, and constants:

```js
/**
 * Acquires a connection from the pool.
 *
 * Creates a new connection if the pool has capacity, otherwise waits up to
 * `acquireTimeoutMs` for one to become available.
 *
 * @returns {Promise<Connection>} A pooled connection.
 * @throws {Error} If the pool is closed or the acquire timeout expires.
 */
async acquire() {
```

Use the following JSDoc tags consistently:

| Tag | When to use |
|-----|-------------|
| `@param {Type} name` | Every function/method parameter |
| `@returns {Type}` | Every function/method with a return value |
| `@throws {Error}` | Document error conditions |
| `@example` | Non-trivial APIs — a short snippet is often more valuable than prose |
| `@see` | Cross-references to related functions, classes, or docs |
| `@deprecated` | Anything scheduled for removal |

### What to Document

Every exported function, class, method, and constant should have at least a one-liner
description. Non-obvious behavior, gotchas, and performance implications deserve
additional explanation.

For classes, document:
* The class purpose (on the `class` declaration)
* Constructor parameters
* All public methods and properties
* Async behavior (what the returned Promise resolves to)

### napi-rs Bridge Documentation

The Rust source files (`src/*.rs`) use `///` doc comments with JSDoc-style tags
(`@param`, `@returns`, `@example`). These comments propagate to TypeScript IntelliSense
via the hand-written `index.d.ts`.

When adding a new napi method in Rust:
1. Write the `///` doc comment with JSDoc tags in the `.rs` file
2. Add the corresponding signature and doc comment to `index.d.ts`
3. The Rust doc and the `.d.ts` doc should match in content

### TypeScript Declarations (`index.d.ts`)

The `index.d.ts` file is **hand-written**, not generated. It must be updated manually
whenever the Rust API changes.

Add TSDoc comments to all declarations. These are the primary source of editor
hover documentation for TypeScript users:

```typescript
/**
 * Executes a SQL query and returns all result rows.
 *
 * @param sql - The SQL query to execute.
 * @returns An array of result rows.
 */
executeQuery(sql: string): Promise<RowData[]>;
```

### ES Module Files (`.mjs`)

Pure JavaScript modules (`pool.mjs`, `arrow.mjs`) should have:
* A module-level JSDoc comment explaining the module's purpose
* JSDoc on every exported function/class/method
* `@example` blocks for primary APIs

### CommonJS Entry Point (`index.js`)

The `index.js` file contains significant logic (platform detection, JS extensions,
utility functions). Document:
* Each JS extension with a comment block explaining what it adds and why
* All exported utility functions with JSDoc
* The platform detection strategy with inline comments


## Naming Conventions

| Element | Convention | Example |
|---------|-----------|---------|
| Classes | PascalCase | `ConnectionPool`, `HyperProcess` |
| Methods/functions | camelCase | `executeQuery`, `getTableNames` |
| Constants | UPPER_SNAKE_CASE | `DEFAULT_TIMEOUT_MS` |
| Private fields | `#` prefix | `#idle`, `#endpoint` |
| Parameters | camelCase | `databasePath`, `createMode` |
| Enums (in .d.ts) | PascalCase name, PascalCase members | `CreateMode.CreateIfNotExists` |


## Async Patterns

* All I/O operations return `Promise`. No callback-style APIs.
* Use `async`/`await` in examples and documentation, not `.then()` chains.
* Document what the Promise resolves to in `@returns`.
* Resource cleanup: support `Symbol.asyncDispose` for `await using` syntax.
  Document this pattern in class-level JSDoc.


## Error Handling

* Throw `Error` with descriptive messages. Do not use custom error classes unless
  the consumer needs to distinguish error types programmatically.
* Document error conditions with `@throws`.
* In napi-rs Rust code, use `Error::from_reason(msg)` to throw JS-visible errors.


## Testing

* Tests use Node.js built-in `assert` module (no external test runner).
* Test files live in `__test__/` as `.mjs` files.
* Each test section should have a descriptive comment header explaining what it tests.
* Benchmarks live alongside tests in `__test__/benchmark.mjs`.


## Documentation Review Checklist

When reviewing documentation changes, verify:

- [ ] All exported functions/classes/methods have JSDoc with at least a one-liner
- [ ] `@param` tags match function signatures
- [ ] `@returns` describes the resolved value for async methods
- [ ] `@throws` documents error conditions
- [ ] `@example` blocks use realistic code (not placeholder values)
- [ ] `index.d.ts` matches the current Rust API surface
- [ ] README does not contain build/publish internals — those belong in DEVELOPMENT.md
- [ ] Cross-references use links rather than duplicating content
- [ ] No stale references to renamed or removed APIs
