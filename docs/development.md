# Development

The workspace builds, tests, and lints with unpinned stable Rust and formats with unpinned nightly
rustfmt. `rust-toolchain.toml` selects stable for `cargo` commands that do not name a toolchain. The
workspace `Cargo.toml` defines the shared lint baseline, `clippy.toml` configures Clippy, and
`.rustfmt.toml` configures rustfmt.

`win-nightlight-lib` and the CLI forbid unsafe code. `win-nightlight-watch` copies the workspace
lint baseline but sets `unsafe_code = "deny"` so individual Win32 wrapper functions can use scoped
exceptions. Keep its lint tables synchronized with the workspace. Each unsafe block needs a `SAFETY`
comment describing handle validity and ownership.

## Setup

Install the toolchains and tools:

```shell
rustup toolchain install stable --profile minimal --component clippy
rustup toolchain install nightly --profile minimal --component rustfmt
cargo +stable install --locked just cargo-audit cargo-machete
```

Install `cargo-insta` to review snapshot changes:

```shell
cargo +stable install --locked cargo-insta
```

`just check-msrv` also requires Bash and `jq`. On Windows, run it from Git Bash so that `cygpath` is
on `PATH`.

Configure the editor to format with nightly rustfmt. For rust-analyzer:

```json
{
  "rust-analyzer.rustfmt.overrideCommand": ["rustup", "run", "nightly", "rustfmt"]
}
```

## Tasks

The `justfile` defines the tasks that local development and CI share:

| Recipe                     | Action                                                            |
| :------------------------- | :---------------------------------------------------------------- |
| `just fmt`                 | Formats the workspace with nightly rustfmt.                       |
| `just lint`                | Checks formatting and runs stable Clippy with `-D warnings`.      |
| `just fix`                 | Applies Clippy fixes and formatting, then runs `just lint`.       |
| `just test`                | Runs all tests, including doctests.                               |
| `just doc`                 | Builds documentation with rustdoc warnings denied.                |
| `just dependencies`        | Runs `cargo audit` and `cargo machete`.                           |
| `just check`               | Runs `lint`, `test`, `doc`, and `dependencies`.                   |
| `just check-msrv PACKAGE`  | Checks `PACKAGE` with the toolchain named by its `rust-version`.  |

Run `just check` before opening a pull request. CI runs each task that `just check` runs as a
separate job, plus `just check-msrv` for all three crates and `cargo publish --workspace --dry-run
--locked`.

## Tests

- Byte fixtures are hex constants: the library's fixtures are in
  `win-nightlight-lib/src/fixtures.rs`, and the CLI and watcher tests define the fixtures they use.
  Expected decoded values come from decoding the fixtures by hand, not from the code under test.
- CLI output tests use [`insta`](https://insta.rs) snapshots in `win-nightlight-cli/src/snapshots/`.
  After a deliberate output change, run `cargo insta test --review` (or `cargo insta review` after
  `just test`) and commit the updated `.snap` files.
- Property tests use `proptest`. When a property test fails, commit the generated
  `proptest-regressions/` file with the fix so the failing case replays in later runs.
- `Nightlight` tests run against an in-memory store with a fixed clock.
- Watcher tests run on Windows and create separate scratch keys under
  `HKEY_CURRENT_USER\Software\win-nightlight-test`, named by test and process ID. Drop guards remove
  each test key. Without changing Night Light, tests cover native notifications, automatic
  registration for the next notification, external waits, decoded snapshots, read failures, deferred
  reads, thread exit, deletion, and handle cleanup.
- The watcher rustdoc example uses Night Light constructors but is marked `no_run`: tests compile it
  without accessing the Night Light registry keys.

## Probes

`probes/` has interactive experiments that change live Night Light settings, such as the [timestamp
rewind probe](../probes/timestamp-rewind/README.md). Each probe is a `publish = false` workspace
member, so `just lint` and `just test` build it and run its unit tests, but neither CI nor any task
in the root `justfile` runs it. Run a probe only through its own `justfile`, with an observer at the
display.

The timestamp rewind probe's trace tools, which summarize its Process Monitor traces, are a Python
uv project in `probes/timestamp-rewind/trace/`. `just --justfile probes/timestamp-rewind/justfile
trace-check` checks its formatting, lint, and types, runs its tests, and audits its dependencies.
The `Lint Python` workflow runs the same task in CI.
