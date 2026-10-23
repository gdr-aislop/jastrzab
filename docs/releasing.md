# Releasing

A release is a tag. Pushing `vX.Y.Z` runs `.github/workflows/release.yml`, which builds a `.deb`,
an AppImage and a Flatpak for arm64/aarch64 and amd64/x86_64, signs them, publishes an **APT
repository** and a **Flatpak repository** to GitHub Pages, and attaches the packages to the
GitHub release.

| Where | What |
|---|---|
| `https://gdr-aislop.github.io/jastrzab/` | the Flatpak repository, and `io.github.gdr_aislop.jastrzab.flatpakrepo` |
| `https://gdr-aislop.github.io/jastrzab/apt/` | the APT repository (`stable main`, `arm64 amd64`), key at `apt/jastrzab.asc` |
| GitHub release `vX.Y.Z` | `.deb`, `.AppImage` + `.zsync`, `.flatpak`, a `.asc` signature for each `.deb` and `.AppImage`, `jastrzab-signing-key.asc`, `SHA256SUMS` |

Both repositories are rebuilt whole by each release and hold only the latest version; older
versions stay on the releases page.

## One-time set-up on GitHub

Do this before the first tag. The workflow cannot do any of it for itself.

1. **The repository must be public.** The workflow builds arm64 packages on the free
   `ubuntu-24.04-arm` runner, which only public repositories get (a private one needs a paid
   larger-runner plan or a self-hosted arm64 runner).

2. **Pages source: Settings → Pages → Build and deployment → Source: GitHub Actions.** Without
   it the `deploy-pages` job fails outright.

3. **Let tags deploy to the `github-pages` environment.** Settings → Environments →
   `github-pages` (it appears after Pages is switched to GitHub Actions; if it is not listed yet,
   create an environment with that exact name) → *Deployment branches and tags* → *Add deployment
   branch or tag rule* → **Ref type: Tag**, **Name pattern: `v*`**. The environment starts out
   restricted to the default branch, and the release runs on a tag, so without this rule
   `deploy-pages` is rejected with *"Tag `v0.1.0` is not allowed to deploy to github-pages due to
   environment protection rules"*.

4. **Signing key: Settings → Secrets and variables → Actions.** Two values, with the same names
   audiobooklet uses (one key can sign both apps; reuse its values if you like):

   | Name | Kind | Value |
   |---|---|---|
   | `FLATPAK_GPG_PRIVATE_KEY` | **Secret** (a *variable* is shown in plain text to everyone who can read the repository, and the workflow does not read variables for this) | the whole armored private key, from `-----BEGIN PGP PRIVATE KEY BLOCK-----` to `-----END …-----` |
   | `FLATPAK_GPG_KEY_ID` | secret or variable, either works (it is public) | the key's 40-hex-digit fingerprint |

   The key must have **no passphrase** and may be **signing-only**; CI imports it non-interactively.
   To make one:

   ```sh
   gpg --batch --passphrase '' --quick-generate-key \
       "Jastrzab release signing <gdr-aislop@users.noreply.github.com>" rsa4096 sign never
   gpg --list-secret-keys --keyid-format long      # note the 40-hex fingerprint
   gpg --armor --export-secret-keys <FINGERPRINT>  # paste this into FLATPAK_GPG_PRIVATE_KEY
   gpg --fingerprint <FINGERPRINT>                 # record it below
   ```

   RSA rather than ed25519, for compatibility with every OSTree and GnuPG a user might have.
   Keep a backup of the private key somewhere safe; delete the local copy if you like.

5. **Actions permissions: Settings → Actions → General.** If you restrict which actions may run,
   allow GitHub-owned actions and **`softprops/action-gh-release@v2`**. *Workflow permissions*
   can stay at "Read repository contents": the `release` job asks for `contents: write`, and
   `deploy-pages` for `pages: write` and `id-token: write`, itself. If an organization policy
   forbids a workflow requesting write permissions, that has to be relaxed for this repository.

## Releasing

1. Set `version` in `[workspace.package]` of `Cargo.toml` (and let `Cargo.lock` follow:
   `cargo check`), add the release to `app/data/io.github.gdr_aislop.jastrzab.metainfo.xml`, and
   commit it.
2. **Dry run first, at least for the first release:** Actions → *Release debs, AppImages, and
   Flatpaks* → *Run workflow* on the branch. It builds all three packages unsigned and skips
   signing, Pages and the release. The `.deb` for the first release also needs the tag to match
   the version, so the dry run is where a mistake in the packaging shows up cheaply.
3. Tag and push:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

   `build-deb` stops at once if the tag and the workspace version differ.
4. When the run is green, check the site and try the install commands from the README on a phone:

   * `https://gdr-aislop.github.io/jastrzab/apt/dists/stable/InRelease`
   * `https://gdr-aislop.github.io/jastrzab/apt/jastrzab.asc`

   The README's APT instructions pin the repository to the single package `jastrzab` with
   `Pin: release o=jastrzab`: that matches the `Origin: jastrzab` line `build-apt-repo` writes into
   the `Release` file, so changing one means changing the other.
   * `https://gdr-aislop.github.io/jastrzab/io.github.gdr_aislop.jastrzab.flatpakrepo`

## The signing key

Fingerprint: *not recorded yet. Paste the output of `gpg --fingerprint` here after step 4.*

## If a run fails

* **`FLATPAK_GPG_PRIVATE_KEY and FLATPAK_GPG_KEY_ID must both be set`** (early in
  *Build Flatpak*): the secrets from step 4 are missing, or the private key was saved as a
  variable instead of a secret.
* **`deploy-pages` is rejected for the tag**: step 3.
* **`deploy-pages` fails with "Not Found" or a permissions error**: step 2.
* **A Flatpak build fails resolving the runtime**: the manifest's runtime (`51`) and the Rust
  extension (`26.08`) in `flatpak/io.github.gdr_aislop.jastrzab.json` and in the *Install
  flatpak-builder* step must stay in step with each other. GNOME 51 is built on freedesktop-sdk
  26.08, GNOME 50 on 25.08.
* **Only one architecture failed**: `combine-flatpak-repo` and `release` need both, so fix the
  failing job and re-run the workflow's failed jobs; nothing is published until both succeed.
