# Development

Use stable Rust to build, test, and lint, and nightly rustfmt to format. Neither toolchain is pinned
to a release. Run `just check` before opening a pull request.

The shared configuration is in:

- `rust-toolchain.toml`: the default toolchain for Cargo commands.
- `Cargo.toml`: workspace lints.
- `clippy.toml` and `.rustfmt.toml`: Clippy and rustfmt settings.

`win-nightlight-lib` and the CLI forbid unsafe code. The watcher uses `unsafe_code = "deny"` so
individual Win32 wrappers can allow it. Keep the watcher's copied lint tables synchronized with the
workspace. Each unsafe block needs a `SAFETY` comment describing handle validity and ownership.

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

CI runs the `just check` tasks as separate jobs. It also checks the minimum supported Rust version
for the published crates and runs `cargo publish --workspace --dry-run --locked`.

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

`probes/` has interactive experiments that change live Night Light settings, including the
[timestamp rewind probe](../probes/timestamp-rewind/README.md).

- Each probe is a `publish = false` workspace member.
- `just lint` and `just test` build probes and run their unit tests.
- CI and the root tasks do not run the live experiments. Run those through the probe's own
  `justfile`, with an observer at the display.

The timestamp probe's Python trace tools are a uv project in `probes/timestamp-rewind/trace/`.
Run `just --justfile probes/timestamp-rewind/justfile trace-check` to check formatting, lint, types,
tests, and dependencies. The `Lint Python` workflow runs the same task in CI.
