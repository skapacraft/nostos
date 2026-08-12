# Submitting to Flathub

Everything here is for the submission. Nothing in this directory is used by
our own builds: `../com.skapacraft.nostos.yml` is what CI compiles on every
commit, and the two differ only in the source, which there is the working tree
and here a tagged commit pinned by its hash.

## What Flathub receives

Their repository holds three files beside each other:

    com.skapacraft.nostos.yml     the manifest in this directory
    cargo-sources.json            generated, see below
    node-sources.json             generated, see below

The generated lists are not committed here, because they run to tens of
thousands of lines and would churn on every dependency bump. They are
committed to the Flathub repository, because a build with no network needs
every dependency declared before it starts, and Flathub will not fetch them
for you.

## Generating the two lists

Run this against the tag the manifest points at, not against `main`. The
generators are the official ones; the commit is pinned for the same reason the
manifest pins the source.

    TOOLS=737c0085912f9f7dabf9341d4608e2a77a51a73a
    curl -fsSL -o /tmp/cargo-gen.py \
      "https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/$TOOLS/cargo/flatpak-cargo-generator.py"
    python3 -m pip install aiohttp toml \
      "git+https://github.com/flatpak/flatpak-builder-tools@$TOOLS#subdirectory=node"

    git checkout v1.1.1
    python3 /tmp/cargo-gen.py src-tauri/Cargo.lock -o cargo-sources.json
    flatpak-node-generator npm package-lock.json -o node-sources.json

The workflow in `.github/workflows/flatpak.yml` does the same thing on every
run, so if it is green the lists generate cleanly from the current lock files.

## Opening the pull request

Flathub takes submissions on a branch of its own, not on `master`:

    git clone --branch=new-pr https://github.com/<you>/flathub.git
    cd flathub
    git checkout -b com.skapacraft.nostos

Put the three files at the root, commit, push, and open the pull request
against the `new-pr` base branch with the title:

    Add com.skapacraft.nostos

## Before submitting

Build it locally first. Flathub asks for this and it is also how you avoid
finding out in review:

    flatpak run org.flatpak.Builder --force-clean --sandbox --user --install \
      --install-deps-from=flathub --ccache --mirror-screenshots-url=https://dl.flathub.org/media/ \
      --repo=repo builddir com.skapacraft.nostos.yml
    flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest com.skapacraft.nostos.yml

None of this runs on macOS. The workflow is there so the answer to "does it
still build" comes from a machine rather than from someone's recollection.

## After a new release

Three things move together, and forgetting the third is the usual mistake:

1. `tag` and `commit` in the manifest
2. the two generated lists, regenerated from that tag
3. a `<release>` entry in `src-tauri/packaging/com.skapacraft.nostos.metainfo.xml`,
   which is in the application repository and ships inside the package

## The application ID

`com.skapacraft.nostos` requires `skapacraft.com` to be reachable over HTTPS,
and Flathub checks it from a datacentre address. Cloudflare's Bot Fight Mode
was challenging exactly those addresses, which made the domain look dead to
every automated check while a browser saw it fine. It is off. If the check
ever fails again, that is the first thing to look at.
