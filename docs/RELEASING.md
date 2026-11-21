# Releasing

Releases are manual: Actions > **Rust workspace** > **Run workflow**, pick the branch, and put
the tag (for example `v0.14.0`) in **Publish a release with this tag**. macOS is built for
releases whether or not the box is ticked.

## Before you start

1. **Merge `main` into the branch you're releasing**, and make sure the branch has everything
   that's meant to be in this release. A release is built from one branch, so work that lives on
   another branch isn't in it.
2. **Set the version** in the workspace `Cargo.toml` (`[workspace.package] version`) to the new
   release, and let `Cargo.lock` follow (`cargo check --workspace`).
3. **Run the check yourself**: `scripts/check-release.sh v0.14.0`. It needs the full history
   (`git fetch --unshallow --tags` once if the clone is shallow).
4. **Lint on current stable Rust.** The workflow uses the newest stable, which gains new Clippy
   lints. Run `rustup update stable`, then `cargo fmt --all --check` and
   `cargo clippy --workspace --all-targets --locked -- -D warnings`. A lint that appears only on
   the CI's newer Rust has already cost one release run.

## What the release check refuses

The workflow runs `scripts/check-release.sh` first, before any build, so a bad release costs
nothing. It refuses a release if:

- the tag already exists, or isn't newer than the latest release;
- the version in `Cargo.toml` isn't the tag's version;
- an earlier release isn't part of the commit (it would **lose features** the last one had);
- `main` has commits the commit doesn't have.

It only warns about other branches that have commits the release doesn't. Branches that are
finished or replaced are listed in `scripts/release-ignore-branches.txt`.

## After

- Open a pull request into `main` **after the release is built, from the same commit**, or
  merge first and release from `main`. If more commits are added to the branch after a pull
  request is merged, they need a new pull request.
- Don't delete old releases: the download buttons always serve the newest one, and older ones
  stay available.
