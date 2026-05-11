# hyperdb-mcp Roadmap

Forward-looking design sketches for features that aren't bugs or tech debt but are worth keeping on the radar. Ordered roughly by expected value vs. implementation effort.

For the current codebase (architecture, design decisions, how to add a tool, known tech debt) see [DEVELOPMENT.md](DEVELOPMENT.md). This file is the inverse: things that **don't exist yet** but we'd like to think about before starting.

Each section follows a loose template: Motivation → Architecture sketch → Estimated size → Risks / open questions → Verdict.

---

## Shared `hyperd` daemon for cross-workspace JOINs

**Motivation.** Today, each MCP server (one per entry in `mcp.json`) spawns its own `hyperd` subprocess via `HyperProcess::new()` in `Engine::new`. Two MCP servers ⇒ two `hyperd` processes ⇒ two fully-isolated databases with no ability to JOIN across them. Users who want to query data that spans two workspaces have to either (a) consolidate everything into one workspace, or (b) manually shuffle data between workspaces via the [export bridge](#raw-fallbacks-for-cross-workspace-data-movement) below.

**Architecture.** Switch the Engine to support *connecting to* an existing `hyperd` rather than always spawning a new one:

1. New CLI flag: `--hyperd-url tab.tcp://localhost:PORT`. When set, skip `HyperProcess::new` and build a `Connection` directly against the given address.
2. One long-lived `hyperd` daemon (outside any MCP server), managed via `launchd` on macOS / `systemd` on Linux / `hyperd &` in a scratch shell.
3. Each MCP server in `mcp.json` gets `--hyperd-url` pointing at the shared daemon plus its own `--workspace` (the per-instance `.hyper` file it manages).
4. Inside the shared `hyperd`, each MCP server sees its own workspace as the default database via `ATTACH DATABASE ... AS workspace`. To JOIN across workspaces, the LLM issues an additional `ATTACH` for the *other* workspace and then uses fully-qualified table names (`other.public.tablename`).

**Estimated size.** ~100–200 LOC:
- `src/engine.rs`: split `Engine::new` into `spawn_hyperd_and_connect` vs. `connect_to_existing`, dispatch on the CLI flag.
- `src/main.rs`: add the `--hyperd-url` flag to the clap parser.
- Reconnection handling: when the shared `hyperd` restarts, detect and re-attach workspaces automatically.
- Health check: `status` should report which mode is active (spawned vs. shared) and the daemon URL.

**Risks / open questions.**
- Port discovery: the shared daemon needs a stable known port, or a discovery file.
- Lifecycle: who starts / stops the shared daemon? Not the MCP server (it might be one of several clients).
- Observability: with many MCP servers sharing one `hyperd`, per-client logs get mixed. May need to rely on `request-id` / `session-id` tagging already in the hyperd log.
- Memory savings on the shared `hyperd` are the motivation, but only really matter once you have ≥3 workspaces — two isn't a big deal on a dev laptop (~150–250 MB idle per `hyperd`).

**Verdict.** Not urgent for current usage (two workspaces). Add when a concrete "I need to JOIN across sandbox + persistent data right now" use case shows up.

---

## Cross-database tools

First-class tools for attaching additional `.hyper` databases and
landing query results into tables. The registry lives on
`HyperMcpServer` and is replayed against a fresh `Engine` whenever
`with_engine` recovers from a ConnectionLost error, so attachments
survive hyperd crashes transparently.

- `attach_database(alias, kind, path, writable?)` — attach a `.hyper`
  file under a chosen alias. `kind="local_file"` is the only kind
  supported today; `"tcp"` (remote hyperd) and `"grpc"` (Data 360)
  are planned. `writable` defaults to `false`; the server's
  `--read-only` flag always wins. The alias `"local"` is reserved for
  the primary workspace.
- `detach_database(alias)` — drops the alias from the registry and
  from the current connection. No-op when the alias is unknown.
- `list_attached_databases()` — enumerates every live attachment
  with its kind, source, writable flag, attach time, and
  (best-effort) a count of visible `public`-schema tables.
- `copy_query(sql, target_table, mode, target_database?, temp_attach?)`
  — runs a read-only SELECT and lands the rows into a target.
  `mode` is explicit: `"create"` errors if the target exists,
  `"append"` errors if it doesn't, `"replace"` drops and recreates.
  `target_database` defaults to the primary workspace; any other
  alias must be attached with `writable: true`. `temp_attach` is
  detached automatically even if the query fails.

Example — JOIN across a scratch `.hyper` file and the primary
workspace, then land the result:

```
attach_database(alias="src", kind="local_file", path="/tmp/scratch.hyper")
copy_query(
  sql="SELECT s.id, s.name, p.amount FROM src.public.customers s JOIN orders p ON s.id = p.customer_id",
  target_table="enriched_orders",
  mode="create"
)
detach_database(alias="src")
```

`_table_catalog` is stamped with `load_tool = "copy_query"` and the
serialized request when the destination is the primary workspace;
attached destinations aren't tracked (their catalog isn't ours).

Rough edges for now: `describe` and the `hyper://tables` resource
still only enumerate tables in the primary workspace, not attached
databases. Use `SELECT * FROM {alias}.pg_catalog.pg_tables WHERE
schemaname = 'public'` as an interim workaround.

---

## Raw fallbacks for cross-workspace data movement

When the four tools above don't cover the use case (other MCP server
owns the file you want to read, remote host, non-Hyper consumer),
keep these workarounds in mind:

1. **`.hyper` → `.hyper` export then load.** Single-table transfer.
   Fastest because `.hyper` is Hyper's native format.
   ```
   # in HyperDB (sandbox)
   export(table="scratch_data", path="/tmp/scratch.hyper", format="hyper")
   # then in HyperDB-persistent
   load_file(table="scratch_data", path="/tmp/scratch.hyper")
   ```
2. **CSV / Parquet / Arrow IPC roundtrip.** Universal fallback —
   works between any two workspaces and between HyperDB and
   non-Hyper consumers. Pays the serialization cost both ways.

---

## `switch_workspace` mid-session tool

Lower-priority than the shared daemon, but conceptually clean: a `switch_workspace(path)` MCP tool that tears down the current Engine and re-instantiates against a different `.hyper` file without a process restart. Also resets the saved-queries store (back to ephemeral/persistent pick), subscription registry, and active watchers.

Useful if you'd rather "flip between N workspaces in one chat" than "have N MCP servers in the sidebar". Feasible as ~100 LOC in `server.rs` but introduces subtleties: what happens to in-flight subscriptions, watcher threads with state, and saved queries whose results reference the old workspace's tables. Probably gated behind a CLI flag so it's opt-in.

Scratched for now — the two-MCP-server pattern (`HyperDB` + `HyperDB-persistent`) covers this use case without adding a new failure mode.

---

## Catalog awareness for attached databases (follow-up to cross-database tools)

The [Cross-database tools](#cross-database-tools) shipped the core
ATTACH / DETACH / copy surface. The remaining piece is teaching the
catalog views about attached databases so the LLM can discover
tables without issuing raw `pg_catalog` SQL:

- `describe` grows an optional `database` parameter to enumerate
  tables under a specific alias (default: primary only).
- `hyper://databases/{alias}/tables` resource — per-attachment
  schema catalog that mirrors the existing `hyper://tables` shape.
- `list_attached_databases()`'s `tables_visible` grows from a count
  into a full name list (cheap because attachments are small and
  queries will rerun `pg_catalog` anyway).

Also punts: remote kinds (`"tcp"` / `"grpc"`) on `attach_database`.
Those need the shared-daemon + credential-profile infrastructure
described above.
