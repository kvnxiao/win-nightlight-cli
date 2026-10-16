# Development

The workspace builds, tests, and lints with floating stable Rust and formats with floating nightly
rustfmt. `rust-toolchain.toml` selects stable for unqualified `cargo` commands. The workspace
`Cargo.toml` defines the shared lint baseline, `clippy.toml` configures Clippy, and `.rustfmt.toml`
configures rustfmt.

`win-nightlight-lib` and the CLI forbid unsafe code. `win-nightlight-watch` copies the workspace
lint baseline with `unsafe_code = "deny"` so individual Win32 wrapper functions can use scoped
exceptions. Keep its lint tables synchronized with the workspace; each unsafe block needs a
`SAFETY` comment describing handle validity and ownership.

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

Run `just check` before opening a pull request. CI runs each task as a separate job, plus
`just check-msrv` for all three crates and `cargo publish --workspace --dry-run --locked`.

## Tests

- Byte fixtures are hex constants: the library keeps them in `win-nightlight-lib/src/fixtures.rs`,
  and the CLI and watcher tests define the fixtures they use. Expected decoded values come from decoding the
  fixtures by hand, not from the code under test.
- CLI output tests use [`insta`](https://insta.rs) snapshots in `win-nightlight-cli/src/snapshots/`.
  After a deliberate output change, run `cargo insta test --review` (or `cargo insta review` after
  `just test`) and commit the updated `.snap` files.
- Property tests use `proptest`. When a property test fails, commit the generated
  `proptest-regressions/` file with the fix so the failing case replays in later runs.
- `Nightlight` tests run against an in-memory store with a fixed clock.
- Watcher tests run on Windows and create separate scratch keys under
  `HKEY_CURRENT_USER\Software\win-nightlight-test`, named by test and process ID. Drop guards
  remove each test key. Tests cover native notifications, automatic rearming, external waits,
  decoded snapshots, read failures, deferred reads, thread exit, deletion, and handle cleanup
  without changing Night Light.
- The watcher rustdoc example uses Night Light constructors but is marked `no_run`: tests compile
  it without accessing those keys.
