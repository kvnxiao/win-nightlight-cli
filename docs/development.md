# Development

The workspace builds, tests, and lints with floating stable Rust and formats with floating nightly
rustfmt. `rust-toolchain.toml` selects stable for unqualified `cargo` commands. The workspace
`Cargo.toml` defines the shared lint baseline, `clippy.toml` configures Clippy, and `.rustfmt.toml`
configures rustfmt.

## Setup

Install the toolchains and tools:

```shell
rustup toolchain install stable --profile minimal --component clippy
rustup toolchain install nightly --profile minimal --component rustfmt
cargo +stable install --locked just cargo-audit cargo-machete
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
`just check-msrv` for both crates and a `cargo publish --dry-run`.
