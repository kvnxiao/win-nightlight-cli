# Releasing

The workspace publishes three crates to crates.io at a shared version:

- `win-nightlight-lib`: Night Light reads and writes.
- `win-nightlight-watch`: registry change notifications.
- `win-nightlight-cli`: the CLI, which installs the `wnl` binary.

Pushing a `v*` tag runs `.github/workflows/release.yml`. The workflow checks that the tag matches
the workspace version and authenticates through crates.io Trusted Publishing. It then checks
each crate on crates.io:

- Existing crates without the tagged version are published through Trusted Publishing.
- Versions that are already published are skipped.
- New crates are skipped with a notice and a reminder in the workflow summary to publish them
  manually and configure Trusted Publishing.
- Network failures, unexpected HTTP responses, and invalid registry responses fail the job.

Cargo publishes dependencies before their consumers. All existing crates selected for publication
must have a Trusted Publisher configured for this repository.

## Prepare and tag a release

1. Set the new version in the root `Cargo.toml` in both places: `workspace.package.version` and
   the `version` of the `win-nightlight-lib` entry under `workspace.dependencies`.
2. Run `cargo check` to update `Cargo.lock`, then validate the workspace:

   ```shell
   just check
   cargo publish --workspace --dry-run --allow-dirty --locked
   ```

3. Merge the release changes into `main`, then tag and push the merged commit. For the `0.3.0`
   release:

   ```shell
   git tag v0.3.0
   git push origin v0.3.0
   ```

4. Confirm the Release workflow succeeds. Check its summary for new crates requiring manual
   publication, and complete the steps below before considering the release finished.
5. Confirm all three crates show the new version on crates.io.

## First publication of a new crate

A crate must already exist on crates.io before its owner can configure
[Trusted Publishing](https://crates.io/docs/trusted-publishing). Publish each new crate once with
an API token; existing crates continue to use Trusted Publishing.

For `win-nightlight-watch 0.3.0`, first push the release tag and let the workflow publish the
library and CLI. The watcher's packaged dev dependency requires `win-nightlight-lib 0.3.0`, so
publish the watcher after that library version is available.

1. Create a temporary API token at <https://crates.io/settings/tokens> with permission to publish
   the new crate, then run `cargo login` and paste the token.
2. From a clean checkout of the `v0.3.0` commit on Windows, publish the watcher:

   ```shell
   cargo publish --package win-nightlight-watch --locked
   ```

3. On crates.io, open **Settings → Trusted Publishing** for `win-nightlight-watch` and add a
   GitHub publisher:

   | Field       | Value                |
   | :---------- | :------------------- |
   | Owner       | `kvnxiao`            |
   | Repository  | `win-nightlight-cli` |
   | Workflow    | `release.yml`        |
   | Environment | `release`            |

4. Revoke the temporary API token at <https://crates.io/settings/tokens> and run `cargo logout`.

Subsequent release tags publish all three crates through Trusted Publishing. Rerunning the
`v0.3.0` workflow skips the versions that are already published.

## Recovery

- **Some crates publish and another fails:** fix the cause and rerun the workflow for the same
  tag. It skips published versions and attempts the remaining existing crates. Complete any
  first publication manually as described above.
- **A published version is broken:** crates.io does not allow a version to be overwritten. Yank
  it with `cargo yank --version <version> <crate>` and release a new patch version.
