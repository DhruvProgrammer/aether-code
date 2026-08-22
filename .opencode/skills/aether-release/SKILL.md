---
name: aether-release
description: "Use when cutting an AETHER release: bumping versions, tagging vX.Y.Z, publishing GitHub releases, or debugging cargo/ring/PowerShell build issues in this repo."
---

# AETHER Release

Cut a release and avoid the three known traps in this repo.

## When to Use
- User says "push to github", "make a release", "new release vX.Y.Z"
- Version bump + tag + GitHub publish is requested
- cargo fails compiling `ring` ("64-bit mode not compiled in")

## Workflow
1. Bump BOTH version files: `Cargo.toml` `[workspace.package] version` AND
   `crates/aether-desktop/tauri.conf.json` `"version"` (missing tauri.conf = installer
   gets named with the old version).
2. Run `cargo check --workspace` to regenerate Cargo.lock; commit lockfile too.
3. Commit, `git push origin main`, `git tag vX.Y.Z`, `git push origin vX.Y.Z`.
4. `gh release create vX.Y.Z --title ... --notes ...`.
5. Confirm both workflows started: `gh run list --limit 4`
   (Release Windows ≈4 min; Desktop Windows ≈15 min produces the NSIS installer).

## Environment Traps
- **cargo needs MinGW64 first on PATH** or `ring` fails with
  "sorry, unimplemented: 64-bit mode not compiled in":
  `$env:Path="C:\mingw64\bin;$env:USERPROFILE\.cargo\bin;"+$env:Path`
- **No bash heredocs**: PowerShell mangles `py - << 'PY'`. Write the script to
  `%TEMP%\opencode\fix.py` via the Write tool, then run `py <path>`.
- **git stderr = PowerShell noise**: `main -> main` in output means success even
  when shown as an error.
- Long builds exceed the default shell timeout — pass timeout ≥300000ms.

## Gotchas
- Old installers accumulate in `target/.../bundle/nsis` via the Actions cache;
  desktop.yml has a clean step — don't remove it.
- If a release ships wrong assets: delete stale assets with
  `gh release delete-asset`, fix, force-move the tag to the fix commit
  (`git tag -f` + push --force) to re-trigger workflows.
