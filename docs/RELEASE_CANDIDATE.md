# Alpha.12 candidate: update, package and rollback (M8)

The alpha.12 candidate is integrated on `main` through [PR #13](https://github.com/Vanir20342222/ytui/pull/13)
and the subsequent integration review. This remains an untagged alpha; see
[the audit tracker](AUDIT_IMPLEMENTATION.md) for public-account/desktop/reliability
gates and [the PR review](PR_REVIEW_2026-10-24.md) for the final integration checks.

## Update this candidate

Close existing ytui processes first. Review the new source, then:

```sh
git fetch origin
git switch main
git pull --ff-only
./install-rust.sh
hash -r
ytui --version
ytui-rs --version
ytui doctor
```

`--version` now includes the build revision. Both launchers must agree. A source
archive without Git metadata reports `unknown`, unless the packager explicitly
sets `YTUI_BUILD_REVISION` to the reviewed full revision. Default installation
follows main; an alpha merge does not publish a tagged stable release.

A changed existing regular launcher is saved as `~/.local/bin/ytui.previous`.
Reinstalling the identical binary preserves that previous snapshot. Legacy
symlink targets are not overwritten. To roll back the executable explicitly:

```sh
tools/rollback-install.sh
hash -r
ytui --version
```

The rollback script swaps the two binaries, so a second invocation rolls forward.
Set `YTUI_INSTALL_DIR` when using a custom installation folder. It does not replace
or downgrade data. Back up configuration and state while the app is closed before
trying any older version. Copy the entire state directory, including SQLite WAL
files when present; never copy a live database as a migration acceptance test.

The candidate adds optional JSON fields with defaults; the SQLite schema stays at
version 2. A restart regression uses an isolated synthetic database. Migration or
rollback of the user's real old data still requires a copied-data acceptance run.

## Prepare an archive

On a clean reviewed checkout with the desired Rust compiler installed:

```sh
tools/package-release.sh /tmp/ytui-candidate-dist
cd /tmp/ytui-candidate-dist
sha256sum -c *.sha256
```

The archive includes both launcher names, Cargo.lock, the app license, dependency
license texts/inventory and compiler/build identity. Tar ordering, owners and
mtime are normalized, and gzip timestamps are disabled. This makes archive
assembly deterministic for identical build inputs. It does **not** claim binaries
built with different compilers, targets or environments are byte-identical.
Dependencies with only SPDX metadata are reported for release review; the
inventory is not a completed legal/compliance acceptance. No tool is bundled or
installed automatically, and the command publishes no tag/release.

The manual `Reviewed release candidate` workflow takes an exact commit and uploads
an archive plus checksum. It cannot publish a GitHub release. Public-network,
real-media and target-desktop acceptance must be reviewed separately before tagging.

## CachyOS / Arch recipe

`packaging/arch/PKGBUILD` is a source-package recipe for the candidate. On Arch:

```sh
cd packaging/arch
YTUI_COMMIT="$(git rev-parse HEAD)" makepkg -s
```

The full reviewed SHA is mandatory; a floating branch/tag is not used as package
source. The recipe is syntax-checked here; `makepkg`/pacman installation requires
a separate Arch/CachyOS acceptance machine. mpv, FFmpeg and Node are dependencies;
external yt-dlp and cliamp are optional. This is not yet an AUR publication.
