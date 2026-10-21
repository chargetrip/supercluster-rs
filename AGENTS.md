# AGENTS.md

A public Rust library that clusters points for maps and other data, based on the supercluster
algorithm.

## Setup and commands

Needs only the Rust toolchain pinned in `rust-toolchain.toml`, and `cargo-tarpaulin` for coverage.
No services or environment variables.

```bash
cargo build                          # compile the crate
cargo test                           # core, no features
cargo test --all-features            # everything, including the doc tests of the README
cargo test --features geojson        # one feature
cargo bench --features geojson       # criterion benchmarks, the slow group included
cargo tarpaulin --tests --bins --all-targets --all-features --locked   # coverage, as in CI

# While you work, run the tests of the code you changed, then the full runs above.
cargo test --lib <filter>            # unit tests
cargo test --test <file> <filter>    # integration tests, in tests/<file>.rs
```

## Features

No feature is on by default: `geojson`, `serde` and `tracing` are opt-in. The crate must build with no
features, with each feature alone and with all of them. Gate code with `#[cfg(feature = "...")]` and
a test file with `#![cfg(feature = "...")]`. An example or benchmark that needs a feature lists it in
`required-features` in `Cargo.toml`.

## Public API and versioning

- Everything `pub` is a promise to users. Add a public item only when a user needs it. Removing or
  changing one is a breaking change.
- Mark the error enum and the structs that only the crate builds `#[non_exhaustive]`, so we can add
  to them later. Derive a trait only when the bounds the derive adds are acceptable. Otherwise write
  the impl by hand: an item holds a reference to a point, so it must not require the point to be
  `Clone`.
- Prefer a closure over a trait for what a user supplies, as `load` and `tile_filtered` do. A
  per-request setting goes on `SuperclusterView` (`with_cluster_max_zoom`), not into a `_with`
  variant of a query.
- Keep `rust-version` in `Cargo.toml` equal to the toolchain in `rust-toolchain.toml` and in the two
  CI workflows, and use no newer language or library feature.
- Do not edit `version` or `CHANGELOG.md`. The release tooling writes both.
- A breaking change gets an entry in the upgrade guide of the README.

## Design constraints

- Points are stored as loaded and never copied. The closure of `load` runs once per point, and
  the index then holds the normalized coordinates, so no user code runs again.
- `KDBush` is private. The tile fixtures list items in the order the KD-tree returns them, so a
  change to the tree must keep that order.
- Only loading logs (feature `tracing`). Queries never log.
- All GeoJSON code lives in `feature.rs`. The core knows nothing about GeoJSON.
- Deserialized data is not validated, so data that the crate did not write can make queries panic.

## Concurrency

The crate does no I/O, spawns no threads and does not depend on an async runtime. Callers decide
where the work runs.

- Keep `Supercluster<P>` `Send` and `Sync` whenever `P` is: no `Rc`, `RefCell`, thread-local or global
  state. A unit test guards this.
- Do not add locks, atomics or parallel iterators to the algorithm without a benchmark that shows
  the gain.

## Memory and performance

The crate must handle millions of points, so keep peak memory and allocations low. The benchmarks
are the reference.

- Reuse buffers in loops that run once per point or per zoom level (see `within_into`). Do not
  allocate per item in `cluster`, in the queries or in `tile_filtered` unless the result needs it.
- Use the smallest type that holds the value (`u32` ids and ranges). Keep `f64` where precision
  matters, such as coordinates and positions.
- Each zoom level keeps its items in a flat `Vec<f64>` with `STRIDE` values per item, and that is
  where most memory goes. A change to the layout needs a measurement before and after.
- Share data through the point type (for example an `Arc`) instead of copying it in the crate.
- Measure a change with `cargo bench --features geojson` before you claim it helps. The group "one
  million points" runs only under `cargo bench`. Do not write a speed or memory number in the docs
  that you have not measured.

## Conventions

- Add a dependency only when std cannot do the job in under 30 lines, and report why. Make it
  optional behind a feature when only one feature needs it. Keep `Cargo.lock` up to date, because CI
  builds with `--locked`.
- The README is the crate documentation, so its Rust code blocks are doc tests. A block that needs a
  feature is marked `rust,ignore`, and the others must compile with no features. Code that cannot
  compile, such as the version 3 code in the upgrade guide, goes in a `diff` block, with `-` lines
  for the old code and `+` lines for the new code.
- Features are additive, because docs.rs builds with all of them (`[package.metadata.docs.rs]`).

## Error handling

- `SuperclusterError` is the only public error. It is `#[non_exhaustive]`, derives `Error` with
  `thiserror` and has a variant only for a failure a caller can see. Build a message from the
  constant it describes (`MAX_ZOOM_LIMIT`).
- Validate input where it enters and return an error: the options in
  `SuperclusterOptionsBuilder::build`, which serde also runs, and the points in `load`. Never panic
  on options or data a caller passes. Points with missing or non-finite coordinates are left out,
  and with the `tracing` feature one `warn` event reports how many.
- Propagate errors with `?`. Don't log an error and return `None` or a default instead. A function
  that cannot fail on valid data returns the value, not a `Result`.
- Indexing may panic only on a broken invariant of the crate itself, never on caller input.
- No `.unwrap()` or `.expect()` in the library. The README, the examples and the doc tests use `?`
  and return a `Result`. Unit and integration tests may unwrap.

## Code style

- Document every item, public or private, because the crate warns on missing docs, in the style of
  the standard library: a summary line, then prose for what the signature does not tell, such as
  what an argument means, what the value holds and which input it rejects. Name arguments in
  backticks. Use no `# Arguments` or `# Returns` sections.
- Give a public function that returns a `Result` an `# Errors` section that names each error, and one
  that can panic a `# Panics` section. Give the main entry points an `# Examples` section, which runs
  as a doc test, uses `?` and ends with `# Ok::<(), supercluster::SuperclusterError>(())`.
- Follow the Rust API guidelines for names: no `get_` prefix on a public query or getter, and
  conversions through `From` where a type wraps another one, like `ClusterId`.
- Write comments like one engineer to another. Explain why, not what changed. Mention a previous
  version only in the upgrade guide of the README.
- Write plain, active sentences in docs and comments, and use the same word for the same thing. Use
  no em dashes or en dashes.
- Less is more: deliver the behavior with as little code as possible. Remove dead and commented-out
  code and anything your change makes redundant.
- Import with `use` rather than fully qualified paths, except for a one-off call. No wildcard imports
  outside tests, except `pub use module::*` in `lib.rs`.
- Don't add `#[allow(clippy::...)]`. If you think you need one, ask first.
- `unsafe` is forbidden (`#![forbid(unsafe_code)]`). If something seems to need it, stop and explain
  why.

## Testing

- Unit tests go in a `#[cfg(test)] mod tests` block at the bottom of the file. Integration tests go
  in `tests/`, one file per topic named `test_<topic>.rs`, and a file that needs a feature starts
  with `#![cfg(feature = "...")]`. Name test functions `test_<function_name>_<scenario>`.
- Test behavior through public functions: assert on outputs and observable effects. Test a private
  function directly only when its logic cannot be reached through the public API.
- Never add production code just to make testing easier. Test-only fixtures and builders live inside
  `mod tests`.
- Compare against fixtures with known-good output (`tests/common`). Don't edit a fixture to make a
  test pass, and first find out why the output changed. The order of results matters, because
  `children`, `leaves` and the items of a tile are compared in order.
- Cover edge cases and failure paths: no points, one point, missing and non-finite coordinates,
  unknown cluster IDs, and `Cartesian` coordinates. Also cover the antimeridian, where a tile at the
  edge of the world shows copies from the opposite edge.
- Check invariants as well as examples: every zoom level counts every point, and the points of a
  cluster are the points of its children.
- A test that sets a global logger gets its own file in `tests/`, because the logger can be set only
  once per process.
- Tests that read a supercluster back from JSON need `float_roundtrip` on the `serde_json`
  dev-dependency, or some numbers change.
- Aim for 90%+ coverage. CI measures it with tarpaulin.

## Definition of Done

Before you report work as done, check each item against your changes (`git status` and `git diff`),
re-reading any section it names. Report every item that isn't fully met, and why.

**Review**

- [ ] Every bullet in **Public API and versioning**, **Features** and **Conventions** holds
- [ ] Every bullet in **Design constraints**, **Concurrency** and **Memory and performance** holds
- [ ] Every bullet in **Error handling**, **Code style** and **Testing** holds
- [ ] The README, the rustdocs and the examples describe what the code does now
- [ ] No functionality is removed and no public API breaks, unless the task requires it
- [ ] Your report lists the public items you changed and marks the breaking changes
- [ ] Security is assessed: no `unsafe`, and no panic on options, IDs or coordinates a caller passes
- [ ] Your report lists new dependencies (with the reason for each), feature changes and CI changes

**Final checks**

Run the checks `CONTRIBUTING.md` documents, the same ones CI runs:

- [ ] Formatting check passes
- [ ] Clippy is clean
- [ ] Tests pass with no features and with all features, doc tests included
- [ ] The docs build with no warnings
- [ ] The crate builds with no features, with each feature alone and with all of them together
- [ ] Changes are left uncommitted and unstaged, ready for human review
