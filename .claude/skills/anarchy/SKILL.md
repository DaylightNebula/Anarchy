---
name: anarchy
description: How to work on the anarchy ECS crate itself (Rust) - where things live, internal gotchas, conventions, and how to test, document and keep the user-facing skills in sync. Use when changing, testing or documenting code in this repository.
---

# Working with anarchy

anarchy is an experimental, highly concurrent ECS. Data is locked per component and per
resource, not per world, so many systems can run through one world at once. The API is
still changing. When something here disagrees with the code, trust the code. `README.md`
is the user guide and has longer examples.

## Map of the crate

| Path | What lives there |
| --- | --- |
| `src/lib.rs` | Crate docs and doctest. Re-exports every module with `pub use x::*`, so users write `use anarchy::*;` |
| `src/worlds.rs` | `World` (a shared handle), `WorldImpl` (entity storage trait), `Entity`, shared `world_tests!` and `test_support` |
| `src/worlds/indexed.rs`, `list.rs` | `IndexedWorld` (default, indexes tables by component) and `ListWorld` (simple scan) |
| `src/worlds/handle.rs` | `EntityHandle`, an id paired with its world |
| `src/hierarchy.rs` | `Hierarchy`, the parent and child links, and `Children` |
| `src/components.rs` | `Component`, `ComponentMeta`, `EntityID`, `ComponentID`, `DynComponent(s)` |
| `src/resources.rs` | `Resource`, `ResourceMeta`, `Res`, `ResMut` |
| `src/queries.rs`, `queries/` | `Query`, `QueryGroup` (tuples), `QueryComponent` (`&A`, `&mut A`, `Option<..>`) |
| `src/events.rs` | `Event<E>` system param, `EventQueue<E>` resource |
| `src/tables.rs`, `tables/` | `Table`, `Cursor`, the lock-free `SingleLinkedListTable` |
| `src/schedules/` | `SystemGraph`, instructions (`before`, `after`, `pin`, ...), `SystemParam`, systems, executors, work queues |
| `src/reflect.rs`, `reflect/` | `Reflect`, `Typed`, `FromValue`, `TypeInfo`; `value` (`Value`), `impls` (std types), `path`, `registry` (`TypeRegistry`), `world` (editing methods on `World`, `ReflectEdit`), `name` (`Name`) |
| `src/scenes.rs`, `scenes/` | `Scene`, patches, `SceneLibrary`; `parse` (text format + `parse_value`), `print` (`to_text`), `spawn`, `save` (`from_world`) |
| `macros/` | `anarchy_macros`: `#[derive(Component)]`, `#[derive(Resource)]`, `#[derive(AsAny)]`, `#[derive(Reflect)]` (`reflect.rs`), `scene!` (`scene.rs`) |
| `benches/query.rs` | Criterion benches for queries, inserts and the hierarchy |

## Using the API

The user-facing skills in `plugins/anarchy/skills/` cover the public API. Read the one
that matches the task:

- `world/SKILL.md`: setup, components, resources, entities, queries, handles, hierarchy
- `systems/SKILL.md`: system params, the graph, executors, events, work queues, custom params
- `scenes/SKILL.md`: reflection, the type registry, editing a world by path, text and
  macro scenes, saving
- `recipes/SKILL.md`: id allocation, spawn helpers, adding and removing components,
  transform propagation, setup functions, game loops, parallel work

## Gotchas

- **Deadlocks.** Locks are not reentrant. A system taking both `Res<R>` and `ResMut<R>` for
  the same `R` deadlocks. So does holding a `resource_mut` or component guard and then
  locking the same thing again. Drop guards before re-locking.
- **The `Children` guard** keeps part of the hierarchy read-locked. Drop it (copy it out
  with `.to_vec()`) before editing the hierarchy on the same thread.
- **Entity ids** are chosen by the caller and never checked for uniqueness. With duplicates,
  the by-id index keeps the last insert, while the tables keep every copy.
- **Entities need at least one component.** Build each one as
  `RelaxedMutex::new(Box::new(c) as DynComponent)`.
- **`Option` terms in `Query::new`** still require the component, so they only resolve to
  `None` with `Query::from_iter` over tables that lack it.
- **System identity is the function's `TypeId`.** Appending the same function again
  replaces it, and `before(f)` / `after(f)` refer to it by type.
- **A failing `SystemParam::extract`**, such as `Res<R>` when there is no `R`, makes the
  system return an error without running, and the executor returns that error.
- **Events expire** after `EVENT_LIFETIME` (1s). A system that runs less often misses them.
  Change it by inserting `EventQueue::<E>::with_lifetime(d)` before anything sends.
- **`Reflect::reflect_info` vs `Typed::type_info`.** The instance method has a different
  name on purpose, since one name on both traits makes `T::type_info()` ambiguous.
- **Statics in generic impls are shared** by every instantiation, so per-type `TypeInfo`
  goes through `TypeInfoCell` (keyed by `TypeId`), never a bare `OnceLock`.
- **The `TypeRegistry` is a resource.** Copy what you need out of it (its function
  pointers are `Copy`) and drop the guard before locking components or calling
  `register_*`, as `reflect/world.rs` does.
- **Auto-registration** goes through `inventory`: `#[derive(Reflect)]` submits an
  `AutoRegistration` for non-generic types, using the autoref probes in `registry.rs` to
  detect components and resources. Every derived type in a test binary is registered, so
  short names like `Health` clash across test modules. Tests that look types up by short
  name insert `TypeRegistry::empty()` and register what they need.
- **Derive macros emit `::anarchy::...`.** Inside this crate that resolves through
  `extern crate self as anarchy;` in `lib.rs`. Don't remove it.

## Changing the crate

Before calling a change done, run all of these:

```sh
cargo test                                          # unit tests + crate doctest
cargo test --features serde                         # serde support for reflection
RUSTFLAGS="-W missing_docs" cargo build             # every public item must stay documented
cargo bench                                         # only when touching hot paths (queries, tables, insert, hierarchy)
cargo build --target wasm32-unknown-unknown --all-targets  # the crate must build for the web
cargo test --target wasm32-unknown-unknown          # runs tests/wasm.rs in Node
```

The wasm commands need rustup with the `wasm32-unknown-unknown` target and
`wasm-bindgen-cli` at the `wasm-bindgen` version in `cargo tree -i wasm-bindgen --target
wasm32-unknown-unknown` (it's the runner in `.cargo/config.toml`). Wasm gotchas:

- **Time:** never use `std::time::Instant`, which panics on wasm32. Use `web_time::Instant`.
- **Threads:** wasm has no threads, so `MultiThreadedExecutor` and anything else that
  spawns threads only work natively. On the web, run graphs with `SingleThreadedExecutor`.
- **Native-only dev dependencies:** `criterion` (via rayon) doesn't build for wasm, so it is
  a native-only dev dependency and `benches/query.rs` compiles to nothing there.
- **Wasm tests:** plain `#[test]`s don't run under the wasm runner. Tests meant for wasm go
  in `tests/wasm.rs` with `#[wasm_bindgen_test]`.

Conventions to follow:

- **Errors:** `anyhow::Result`, using `bail!` / `.context(..)`. Fallible public functions
  get a `# Errors` doc section.
- **Docs:** every public item gets a `///` doc, and every module a `//!` doc. Keep them
  short and say what it does, not how. Link types with intra-doc links (`[`World`]`).
  Document private fields and helpers when their purpose isn't obvious from the name.
- **Style:** `impl <T: Bound> Trait for X` (with a space after `impl`), terse one-line
  bodies where they fit, and comments only for non-obvious reasons (lock ordering, why a
  guard is dropped early, etc.).
- **Tests** live in a `#[cfg(test)] mod tests` at the bottom of each file. Use
  `crate::worlds::test_support::*` for the test components `A`, `B`, `C`, `D` (each wraps a
  `u32`) and the helpers `comps`, `by_mask`, `entities` and `value_of`.
- **A new `WorldImpl`** should implement `test_support::TestWorld` and invoke
  `world_tests!(YourWorld)` to get the shared behaviour tests.
- **Concurrency:** anything shared must be `Send + Sync`. Prefer lock-free or sharded
  structures (`ArcSwap`, `DashMap`, `crossbeam` queues) over a single big lock. Hold a lock
  only for as long as you need it.
- **New public features** go in the README too: a bullet under Features, a section in the
  Guide and a row in "Concepts at a glance". Check README snippets compile.
- **Keep the plugin skills in sync.** Users of the crate install `plugins/anarchy` as a
  Claude Code plugin (the marketplace lives in `.claude-plugin/marketplace.json`). When a
  public API changes, update the matching skill under `plugins/anarchy/skills/`. To check a
  skill's snippets, paste them into a throwaway `tests/*.rs` file, run `cargo test`, then
  delete the file. Run `claude plugin validate .` after editing any plugin file.
