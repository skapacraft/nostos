# Contributing to Nostos

This is maintained by one person, so the most useful contribution is usually a
precise report rather than a large patch. Everything below exists to make a
change reviewable, not to add ceremony.

## Before anything else

A security problem does not belong in a public issue. Use the **Security** tab,
then **Report a vulnerability**. What counts as one here is in
[SECURITY.md](SECURITY.md).

## Reporting a bug

Open an issue with the bug template. What makes a report actionable:

- the exact steps, from a clean state, and what you expected instead
- the version, and the platform it ran on
- a file or a screenshot when the trigger is one specific input

If the input holds personal data, describe how to build an equivalent one
rather than attaching it.

## Suggesting a feature

Open an issue with the feature template and describe the problem before the
solution. A feature that does not pull its weight does not ship: that is the
project's standard, not a rejection of the idea.

## Pull requests

Open an issue first for anything beyond a typo or a one-line fix, so the design
is agreed before the work happens.

Once that is settled:

1. Branch from `main`.
2. Keep the change to one concern. Two unrelated fixes are two pull requests.
3. Add a `CHANGELOG.md` entry, and bump the version where the project keeps it.
4. Verify it, and say in the pull request how you did.

## Building and checking locally

```bash
npm install
npm run tauri dev
```

CI runs these on every push, and a pull request has to pass all of them:

```bash
cd src-tauri
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

The tests run on synthetic trees built in a temporary folder, so they need no
Takeout export of your own. A handful are `#[ignore]`d because they want a real
archive; CI runs those separately.

## The constraint that is not negotiable

**This application opens no network connections.** That is not a policy, it is
enforced: the `Local-first constraint` job in CI runs `cargo deny` over the
dependency graph and fails the build if an HTTP client, a TLS stack or a
telemetry crate appears anywhere in it, however indirectly. A pull request that
introduces one will not go green, and reworking it to pass by relaxing the check
is not the fix.

The same applies to the rest of what [PRIVACY_AUDIT.md](PRIVACY_AUDIT.md)
documents: the Content Security Policy, the absence of an updater, the minimal
capability set. If a change needs one of those loosened, that is the thing to
open an issue about, before writing the code.

## The other rule: nothing is ever deleted

No function in this application removes a file. Cleanup builds an alternative
tree or moves files to quarantine; repair writes a copy, or rewrites in place
after checking there is room. An export is often the only copy left of the
photographs in it, so a change that could delete, truncate or overwrite one will
be rejected however convenient it is.

## House rules

- **English only**, in code comments, commit messages and user-facing strings.
- **Commit messages** say what changed and why. The subject line is imperative
  and under 72 characters, the body wraps at 72 and explains the reasoning that
  is not obvious from the diff.
- **No generated or vendored files** in a commit unless the project already
  tracks them.
- **No credentials, keys or personal data**, including in test fixtures.

## Licence

By contributing you agree that your work is distributed under the licence in
[LICENSE](LICENSE).
