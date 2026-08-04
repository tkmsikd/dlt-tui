# Releasing

Steps to cut a new dlt-tui release.

## 1. Prepare the release

1. Bump the version in `Cargo.toml` (`[package] version = "X.Y.Z"`).
2. Add a new entry at the top of `CHANGELOG.md` following the existing
   [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format.
3. Run the same checks as CI and verify the crates.io package before creating a tag:

   ```bash
   cargo fmt --check
   cargo clippy --all-targets --locked -- -D warnings
   cargo test --all-targets --locked
   cargo publish --dry-run --locked
   cargo package --list
   ```

   Review the package list for local files, secrets, and generated artifacts.
4. Commit both changes:

   ```bash
   git add Cargo.toml CHANGELOG.md
   git commit -m "release: vX.Y.Z"
   ```

## 2. Tag and push

```bash
git tag vX.Y.Z
git push origin master
git push origin vX.Y.Z
```

Pushing the tag triggers `.github/workflows/release.yml`, which:

1. **`verify`** — checks the tag/version match, formatting, Clippy, tests, the
   crates.io package, and the Rust 1.88 MSRV before anything is published.
2. **`create-release`** — creates the GitHub release with notes pulled from `CHANGELOG.md`.
3. **`upload-binaries`** — builds and uploads archives for all six targets
   (Linux x86_64 gnu/musl, aarch64 musl; macOS x86_64/aarch64; Windows x86_64).
4. **`update-tap`** — downloads the four Homebrew-relevant archives
   (macOS x86_64/aarch64, Linux musl x86_64/aarch64), computes their SHA-256
   checksums, regenerates `Formula/dlt-tui.rb` in
   [tkmsikd/homebrew-tap](https://github.com/tkmsikd/homebrew-tap), and pushes
   the update to `main`.

Watch the [Actions tab](https://github.com/tkmsikd/dlt-tui/actions) until all
four stages go green. If `update-tap` fails (e.g. an asset 404s because a build
job failed), fix the underlying issue and re-run the workflow — it's safe to
re-run since it always regenerates the formula from scratch.

## 3. Publish to crates.io

```bash
cargo publish --locked
```

## 4. Verify

- `brew update && brew upgrade dlt-tui` (or a fresh `brew install
  tkmsikd/tap/dlt-tui`) picks up the new version.
- `cargo install dlt-tui --force` picks up the new version from crates.io.

---

## One-time setup: `TAP_GITHUB_TOKEN`

The `update-tap` job needs push access to `tkmsikd/homebrew-tap`, which the
default `GITHUB_TOKEN` (scoped to `tkmsikd/dlt-tui`) can't provide.

1. Create a fine-grained personal access token at
   <https://github.com/settings/personal-access-tokens/new>:
   - **Resource owner**: `tkmsikd`
   - **Repository access**: only `tkmsikd/homebrew-tap`
   - **Permissions**: Contents → Read and write
2. In `tkmsikd/dlt-tui` → Settings → Secrets and variables → Actions, add a
   repository secret named `TAP_GITHUB_TOKEN` with the token value.
3. Re-run this whenever the token expires or is rotated.
