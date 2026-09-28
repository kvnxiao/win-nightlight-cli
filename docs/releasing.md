# Releasing

The workspace publishes two crates to crates.io at a shared version:

- `win-nightlight-lib`: the library.
- `win-nightlight-cli`: the CLI, which installs the `wnl` binary.

Pushing a `v*` tag runs `.github/workflows/release.yml`. The workflow checks that the tag matches
the workspace version, authenticates through crates.io Trusted Publishing, and runs
`cargo publish --workspace`. Cargo publishes the library before the CLI.

## First release

Trusted Publishing only works for crates that already exist on crates.io, so the first version is
published by hand with an API token.

1. Create an API token at <https://crates.io/settings/tokens> with the `publish-new` and
   `publish-update` scopes, then run `cargo login` and paste the token.
2. From an up-to-date `main`, publish both crates:

   ```shell
   cargo publish --workspace --locked
   ```

3. On crates.io, open **Settings → Trusted Publishing** for each crate and add a GitHub publisher:

   | Field       | Value                |
   | :---------- | :------------------- |
   | Owner       | `kvnxiao`            |
   | Repository  | `win-nightlight-cli` |
   | Workflow    | `release.yml`        |
   | Environment | `release`            |

4. Tag the published commit:

   ```shell
   git tag v0.1.0
   git push origin v0.1.0
   ```

   The Release workflow runs and fails at the publish step with `already exists on crates.io index`,
   because step 2 already published this version. This failure is expected for the first tag only.

5. Revoke the API token at <https://crates.io/settings/tokens>.

## Subsequent releases

1. Set the new version in the root `Cargo.toml` in both places: `workspace.package.version` and
   the `version` of the `win-nightlight-lib` entry under `workspace.dependencies`.
2. Run `cargo check` to update `Cargo.lock`, then confirm both crates package and verify:

   ```shell
   cargo publish --workspace --dry-run --allow-dirty
   ```

3. Commit `Cargo.toml` and `Cargo.lock` to `main`, then tag and push:

   ```shell
   git tag v0.2.0
   git push origin v0.2.0
   ```

4. Confirm the Release workflow succeeds and both crates show the new version on crates.io.

## Recovery

- **The library uploads but the CLI fails:** re-running the workflow fails because the library
  version already exists. Fix the cause, then publish only the CLI from the tagged commit with an
  API token: `cargo publish -p win-nightlight-cli --locked`.
- **A published version is broken:** crates.io does not allow a version to be overwritten. Yank it
  with `cargo yank --version <version> <crate>` and release a new patch version.
