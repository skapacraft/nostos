# Privacy and security audit

This document describes the offline architecture of Nostos and how to
verify it yourself. It does not ask for trust: every claim comes with the
command that confirms or refutes it.

Last verified: 2026-08-05, on the initial commit.

## 1. The constraint

Nostos handles the archives a person downloads precisely because they
want their own data back. If the application contacted a server, any server, for
any reason, it would defeat the point. Hence a single rule, which overrides
every consideration of convenience:

> The application process opens no network connections.

## 2. How the constraint is made executable

A promise written in a README ages badly at the first `cargo add`. The
constraint is therefore encoded in `src-tauri/deny.toml`, section `[bans]`, and
enforced in CI by the `Vincolo local-first` job.

Network crates are banned by name (`reqwest`, `hyper`, `ureq`, `curl`, `axum`,
`tungstenite`, `quinn`), along with TLS stacks (`rustls`, `native-tls`,
`openssl`), telemetry (`sentry`, `opentelemetry`) and the Tauri plugins that
would open surfaces to the outside (`tauri-plugin-http`, `tauri-plugin-updater`,
`tauri-plugin-opener`, `tauri-plugin-shell`).

Verify with:

```bash
cargo deny --manifest-path src-tauri/Cargo.toml check
```

The ban has been tested the other way round: adding `reqwest` to the
dependencies makes `cargo deny check bans` fail, reporting three forbidden
crates (`reqwest`, `hyper`, `hyper-util`). It is not a decorative check.

On the frontend side CI rejects `fetch(`, `XMLHttpRequest`, `WebSocket` and
`EventSource` in the sources. The CSP would block them at runtime anyway, but a
failing build is a better diagnosis than a console error.

## 3. Something that looks like a violation and is not

**`reqwest` appears in `src-tauri/Cargo.lock`.** Anyone doing a quick audit will
find it with a `grep` and conclude the app phones home. It does not, and it is
right to explain why rather than leave the doubt standing.

`tauri` declares `reqwest` as an **optional** dependency, activated only by the
`native-tls` and `rustls-tls` features, which this project does not enable. The
lockfile records the entire universe of resolvable dependencies, including the
ones that are never compiled.

Three independent checks:

```bash
# 1. Not in the active dependency graph: prints "nothing to print".
cargo tree --manifest-path src-tauri/Cargo.toml --edges normal -i reqwest

# 2. cargo-deny evaluates the features actually enabled, and passes.
cargo deny --manifest-path src-tauri/Cargo.toml check bans

# 3. The shipped binary imports not one networking symbol.
BIN="src-tauri/target/release/bundle/macos/Nostos.app/Contents/MacOS/nostos"
for s in _socket _connect _getaddrinfo _bind _listen; do nm -u "$BIN" | grep -c "^$s\$"; done  # all 0
```

The third is the hardest to fake, and it holds for the **release bundle**, not
just the development build: the binary does not even import `socket`, `connect`
or `getaddrinfo` from the system library.

Verified at runtime as well, with the application launched from the bundle and
no development server running:

```bash
lsof -a -p "$(pgrep -f 'Nostos.app/Contents/MacOS')" -i -P -n   # no rows
```

Mind the filter: `lsof -p PID -i` combines the two conditions with OR and would
end up listing the sockets of every system daemon. The `-a` is required.

## 3b. Another string that looks like a violation

The release binary contains `ws://localhost:1420`, the address of Vite's hot
reload. This one is inert too.

Tauri embeds the entire configuration in the binary, `devCsp` included, but it
chooses between them like this (`tauri/src/manager/mod.rs`):

```rust
fn csp(&self) -> Option<Csp> {
  if !crate::is_dev() { self.config.app.security.csp.clone() }
  else { self.config.app.security.dev_csp.clone().or_else(...) }
}
```

And `is_dev` is a **compile-time constant**, not a runtime check:

```rust
pub const fn is_dev() -> bool { !cfg!(feature = "custom-protocol") }
```

`tauri build` enables `custom-protocol`, so in the distributed package the
`devCsp` branch is dead code eliminated by the compiler. This is not a
configuration matter that someone could flip at startup.

The practical proof: the bundle launched with no development server works, which
means it is serving the embedded assets and not `http://localhost:1420`.

## 4. An honest perimeter: the webview

A declaration like this one would be dishonest if it stopped at the Rust code.
The application embeds the system webview (WKWebView on macOS, WebKit2GTK on
Linux, WebView2 on Windows), which is an operating system component and, in the
abstract, knows how to speak over the network.

What contains it is the Content Security Policy declared in
`src-tauri/tauri.conf.json`, which in production reads:

```
default-src 'self'; script-src 'self'; connect-src 'self' ipc: http://ipc.localhost;
object-src 'none'; frame-src 'none'; form-action 'none'
```

`connect-src` admits only the local IPC channel towards the Rust backend. There
are no permitted remote origins, `form-action` is forbidden and there is no
frame. The content loaded is exclusively what is packaged in the bundle: no CDN,
no remote fonts, no external images.

In development a separate `devCsp` applies, reopening only
`ws://localhost:1420` for Vite's hot reload. That is not the policy that ends up
in the distributed binary.

## 5. The surface granted to the frontend

The React code has no direct filesystem access. The window capability, in
`src-tauri/capabilities/default.json`, grants three entries and no more:

| Permission | Why |
| --- | --- |
| `core:default` | events, window, drag and drop, menu |
| `dialog:allow-open` | file and folder picker |
| `dialog:allow-save` | save picker for exports |

Every read and every write goes through an explicit, named Rust command. There
is no permission that would let the frontend read an arbitrary path, open a URL
or run a process of its choosing.

## 6. Data written to disk

| What | Where | When |
| --- | --- | --- |
| Analysis results | process memory | until it closes |
| Files extracted from an archive | folder chosen by the user | only on an explicit action |
| Media modification dates | the original files | only on an explicit action |
| Files moved to quarantine | folder chosen by the user | only on an explicit action |
| The quarantine ledger | inside the quarantine itself | together with the move |
| `preferences.json` | system configuration folder | only if you tick "do not show again" |

### The only file the app writes for itself

`preferences.json`, in the system configuration folder, holds a single boolean
field:

```json
{ "hideWelcome": true }
```

It exists to remember that you ticked "do not show again" in the first-run
introduction. It is created **only** if you tick that box: leave it alone and
the file never exists.

It contains no paths, no history, no identifiers. The struct in `app_state.rs`
has exactly one field, and any field added in future has to be declared here:
that is why the comment above that struct says so explicitly.

No caches, no on-disk logs, no history of opened paths and no installation
identifiers are written.

Development diagnostics go to stderr and are wrapped in
`#[cfg(debug_assertions)]`: in the distributed binary those lines do not exist.
It is deliberately not a file logger.

The quarantine ledger is written inside the folder the user has just chosen,
alongside the files that were moved. It holds their original paths, and without
it the operation would not be reversible.

## 7. Other absences

- **No auto-updater.** `createUpdaterArtifacts` is disabled and the plugin is
  not installed. Updates are downloaded by hand from the releases.
- **No crash reporter.** A panic stays on the machine.
- **No clickable links to the outside.** URLs found in Google Drive
  placeholders are shown as text. Opening them would mean a connection to
  Google, and that is a decision for the user to make outside this application.

## 7b. The two actions towards the operating system

**Revealing a path.** The "Show in Finder" button invokes an external program:
`open -R` on macOS, `explorer /select,` on Windows, `xdg-open` on the folder on
Linux.

The constraints are deliberately tight:

- the program invoked is **fixed in the code**, not a string anyone could
  influence;
- the only argument is a path that must **already exist** and that is
  canonicalised before use;
- it does not go through a shell, so there is no command injection;
- on Linux it opens the **folder** and not the file, because `xdg-open` on a
  file would open it with the default application, which is a different thing
  from revealing it.

**Opening a problem report.** The "Report a problem" button hands a `mailto:`
to the system, which opens the user's own mail client on a pre-filled message.

This is not a network connection made by this process. Nothing leaves the
machine until the user presses send in an application that is not this one, and
the whole text is on screen before that: the report is shown in full, and can be
edited, precisely because a privacy tool that sent diagnostics its user had not
read would be a contradiction.

The constraints match the ones above:

- the address is a **compile-time constant**, `support@skapacraft.com`. Nothing
  in an archive, a filename or a preference can redirect a report elsewhere;
- subject and body are percent-encoded down to the unreserved set of RFC 3986,
  which is what stops a newline in a path from closing the `subject` field and
  opening a `bcc`. The test `the_mailto_encoding_leaves_no_way_to_inject_a_header`
  fails if that encoding is ever loosened;
- it does not go through a shell. On Windows the URL goes to
  `rundll32 url.dll,FileProtocolHandler` rather than `start`, which would need
  one;
- the home directory is replaced with `~` before the text is composed, so the
  account name does not travel with the paths that failed.

The report holds the version, the operating system, the architecture, what the
user typed, and the error messages seen during the session. Those messages are
kept in memory and bounded to the last twenty: nothing is written to disk unless
the user chooses to save the report, in which case the path comes from the save
dialog like every other export.

`tauri-plugin-opener` and `tauri-plugin-shell` remain banned in `deny.toml`: the
first can also open arbitrary URLs in a browser, the second can run arbitrary
commands. What is here is one fixed address and one fixed program, which is a
different thing. Neither action involves a connection made by this process, so
neither dents the promise in section 1.

## 8. Known and accepted advisories

`cargo deny check advisories` treats vulnerabilities as blocking errors, while
*unmaintained* advisories apply to direct dependencies only
(`unmaintained = "workspace"`).

The reason: fifteen *unmaintained* advisories come from the GTK3 bindings, which
Tauri requires on Linux, and from crates internal to `tauri-utils`. They
describe no exploitable flaw and there is no safe upgrade. Listing them by hand
would produce a list to renew at every Tauri release, and a list that gets
updated out of habit will sooner or later cover the advisory that matters.

### Two vulnerabilities with a documented exception

`quick-xml` 0.37.5, pulled in by `little_exif` (the library that writes the EXIF
tags), has two known denial of service issues: RUSTSEC-2026-0194 (quadratic time
on duplicate attributes) and RUSTSEC-2026-0195 (unbounded allocation of
namespace declarations). There is no fixed 0.37.x and `little_exif` exposes no
feature to disable XMP.

They are listed under `ignore` for a verified reason, not for convenience:
inside `little_exif` that XML parser is used only by `xmp.rs`, which is
reachable only from the PNG writing path. This project excludes PNG from
`EXIF_WRITABLE_EXTENSIONS`, so the vulnerable code is never executed.

The exclusion is not left to the memory of whoever writes the next commit: the
test `png_stays_out_of_exif_writing` fails if anyone adds PNG to the
list, and its message points back to this exception.

The exception should be removed once `little_exif` moves to `quick-xml` 0.41.

Apart from these two, no known vulnerability was present at the time of
verification.

## 9. Redoing the audit

```bash
cargo deny --manifest-path src-tauri/Cargo.toml check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm audit --audit-level=high
```

If any of these fails, the promise made by this document no longer holds.
