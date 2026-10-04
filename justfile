clippy_scope := "--workspace --all-targets --all-features --locked"

fmt:
    cargo +nightly fmt --all

lint:
    cargo +nightly fmt --all --check
    cargo +stable clippy {{clippy_scope}} -- -D warnings

fix:
    cargo +stable clippy --fix {{clippy_scope}} --allow-dirty
    just fmt
    just lint

test:
    cargo +stable test --workspace --all-features --locked

doc:
    RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -D warnings" cargo +stable doc --workspace --all-features --no-deps --locked

dependencies:
    cargo +stable audit
    cargo +stable machete

check: lint test doc dependencies

[positional-arguments]
check-msrv package:
    #!/usr/bin/env bash
    set -euo pipefail
    metadata=$(cargo +stable metadata --no-deps --format-version 1 --locked)
    msrv=$(jq -er --arg name "$1" '
        .workspace_members as $members
        | .packages[]
        | select(.id as $id | $members | index($id))
        | select(.name == $name)
        | .rust_version // error("selected package must declare rust-version")
    ' <<< "$metadata")
    rustup toolchain install "$msrv" --profile minimal
    cargo +"$msrv" check --package "$1" --all-targets --all-features --locked
