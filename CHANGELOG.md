# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the numbering follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- A Mac App Store build, with the packaging the Store requires: App Sandbox,
  an embedded provisioning profile and a signed installer package. Building it
  is `scripts/mac-app-store.sh`, because Tauri's bundler stops at the `.app`.
- The Store build, and only that build, requests
  `com.apple.security.network.client`. App Sandbox is mandatory there and
  WKWebView will not start inside it without that permission: the WebContent
  process is refused at launch and the window stays empty. Measured from one
  binary signed twice, 1836 WebContent restarts in nine seconds without it and
  none with it. No networking library enters the dependency graph, the check
  that enforces that still runs on every change, and the content security
  policy still admits no remote origin. What changes is that on this one build
  the sandbox is no longer what makes a connection impossible. Section 4b of
  PRIVACY_AUDIT.md documents it.
- Extraction of a loaded `.zip` Takeout export, from a panel in the source
  step. The backend commands for it existed already; nothing had ever called
  them, so a `.zip` always ended on "not extracted".
- `Square300x300Logo.png`, the one tile size missing from the MSIX set, and
  `scripts/capture-app-store-screenshots.sh`, which standardises the four Mac
  App Store screenshots at 1440x900 without window automation.

### Fixed
- Two bugs found while testing extraction live: the reported destination
  pointed at the folder the user picked rather than the Takeout root actually
  written inside it, and `__MACOSX` junk left by zipping on a Mac broke that
  root detection entirely.
- The About window's icon was decoded once ahead of time from its own copy of
  the artwork rather than read from a PNG at runtime, so the margin fix above
  passed over it: every other rendered size moved to the 0.875 ratio and this
  one kept the old full-bleed art. Regenerated from the same source.

### Changed
- The in-app guide no longer links to GitHub releases, now that distribution
  goes through the Mac App Store and Microsoft Store: Apple's review note
  2.4.5(vii) rules out pointing at an update mechanism outside the Store.
- The Flatpak workflow now runs by hand only. It used to fire on nearly every
  push to `main`, since `src-tauri/**` and `src/**` cover almost every change,
  duplicating what CI already checks on the same commit.

## [1.2.0] 2026-08-27

### Added
- The interface now follows the device's language instead of always speaking
  English. English and Italian for now; anything else falls back to English
  rather than mixing an unfinished language in. The native menu picks it up
  the same way, read from the OS locale via `sys-locale` since it is built
  before any web view exists and `navigator.language` is not reachable there.
- A new icon: an orbit that has not closed yet, with the piece travelling
  back to complete it. Colour now follows the site's accent, `#7f9cff` on
  `#0b0d10`, rather than the emerald the icon shipped with originally.

### Fixed
- Translating surfaced six strings left in Italian since before the English
  migration and never caught: the whole progress bar, the export-button busy
  state, a file dialog filter name, a default button label, and the fallback
  error message shown when nothing more specific applies.
- The native "About" window showed the generic system folder icon, because
  no icon had ever been set on it.
- The new icon filled its canvas edge to edge, which left it sitting larger
  than every neighbour in the Dock. Apple leaves a margin: Notes, Reminders,
  Calculator, Preview and Music all draw their square at 224 of a 256 canvas,
  and the icon now uses the same ratio. Windows tiles gain from it too, where
  the content area is about two thirds of the tile.

## [1.1.1] 2026-08-12

Packaging only. Nothing about what the application does has changed.

### Added
- An AppStream metainfo file and a desktop entry, both installed by the `.deb`.
  Before this a software centre had a filename to show and nothing else.
- A Flatpak manifest and the workflow that builds it. Flathub compiles with no
  network, so every crate and every npm package is declared with a URL and a
  checksum, generated from the lock files on each run. The sandbox is granted
  neither network nor filesystem access.
- The file dialog backend is a build-time choice. GTK stays the default for the
  `.deb`, the `.rpm` and the AppImage; the Flatpak selects the XDG portal, which
  hands back the folder the user picked instead of their whole home directory.

### Fixed
- The package description was still in Italian. It is what `apt show` prints and
  what the Windows installer displays.


## [1.1.0] 2026-08-10

One panel and the tidying that had accumulated since 1.0.0.

The panel answers the question a program that never phones home cannot ask on
your behalf: is the copy you are running still current. It answers it from the
compile date and the clock on the desk, which is the only honest way to answer
it without opening a connection.

### Fixed
- CI was red on `main`. `cargo fmt --all --check` had been failing since the
  1.0.0 commit, on three call sites in `lib.rs` that rustfmt wanted on one line.
  Formatting applied; clippy and the 72 tests were green throughout.
- The album manifest carried its `note` field in Italian, and ungrammatically at
  that ("una copy della photos"). It is a string this application writes into a
  file the user is left holding, so it now reads as English prose and says what
  the file is for.
- Four other pieces of Italian left in the source: a comment in `calendar.rs`,
  two test identifiers, an assertion message in `drive.rs`, and a comment in
  `index.css`.

### Added
- **Version and updates**, from the Help menu. It states the version, the date
  the binary was compiled and the address where new versions are published, and
  past six months it says outright that the copy is old. Everything is worked
  out on the machine: `build.rs` plants the compile timestamp in the binary and
  the panel subtracts it from the local clock. No server is asked anything,
  because an update check carries an address, an hour and a version to whoever
  answers it, which is the trail this application is built not to leave. Where a
  store or a package manager installed Nostos, that is what updates it.
- `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and issue and pull request templates,
  matching the other SkapaCraft repositories. The contributing guide states the
  two rules that are not up for negotiation here: the application opens no
  network connections, which `cargo deny` enforces in CI rather than documenting,
  and no function deletes a file.

### Changed
- `package-lock.json` had been declaring version `0.1.0` since before 1.0.0,
  where `package.json`, `Cargo.toml` and `tauri.conf.json` all agreed on the
  real number. The four now say the same thing.

## [1.0.0] 2026-08-09

First published version.

The number is 1.0.0 and not 0.x because this is what gets distributed: on a
store, a leading zero reads as abandoned rather than as modest. It does what
the sections below describe, on the systems CI starts it on.

Ships for Windows and Linux, both built and started by CI on every commit.
macOS: to be confirmed.

### Added

- **Report a problem**, from the Help menu. The application cannot send
  anything: `tauri-plugin-http` and `tauri-plugin-opener` are both banned, so
  there is no API to post to and no browser to open. It therefore prepares the
  report and the person carries it, by mail to `support@skapacraft.com` with a
  subject already filled in, or as a file saved wherever they choose.
- The text is shown in full and can be edited before anything happens. Paths are
  redacted, with the home directory replaced by `~`, and the errors seen during
  the session are kept in memory, bounded to the last twenty, so nothing new is
  written to disk.
- The `mailto:` handoff is documented in section 7b of `PRIVACY_AUDIT.md`
  alongside the file manager button, with the same constraints: a compile-time
  address, no shell, and percent-encoding strict enough that a newline in a path
  cannot open a mail header. The test
  `the_mailto_encoding_leaves_no_way_to_inject_a_header` guards that.

### Changed

- The backend no longer composes user-facing sentences. Warnings, errors,
  section and category labels, sidecar retention reasons and privacy notes all
  travel as codes plus the numbers needed to build the message, and the wording
  is chosen in `src/lib/messages.ts`. Translating only the frontend would have
  produced an English application emitting Italian phrases, and every new
  language would have meant going back into the Rust.
- Errors cross the IPC channel as an `ErrorPayload` object with a code and its
  data, instead of being flattened to a string.
- The maps in `messages.ts` are `Record`s over the full union of codes and the
  switches have no default branch, so adding a variant in Rust without the
  matching text fails the frontend build rather than showing a raw code on
  screen.
- Repository documentation, source comments and identifiers are in English.
- `cargo deny` now reports unsound advisories (`unsound = "all"`). It did not
  before: with `version = 2` it reports vulnerabilities and stays silent on
  informational advisories unless asked, which is how RUSTSEC-2024-0429 against
  `glib` reached Dependabot while this check said "advisories ok". The advisory
  cannot be fixed here, since Tauri 2 requires `gtk ^0.18` and the first patched
  `glib` is 0.20.0; the reasoning sits next to the exception in `deny.toml`.
  Test names, local variables, the helper script and the environment variables
  of the manual measurements were renamed: `FOTO` is now `PHOTOS`, `SERIE` is
  `SERIES`, `USCITA` is `OUTPUT`, `CARTELLA` is `FOLDER`, and
  `tools/genera_serie_takeout.py` is `tools/generate_takeout_series.py`.
- The folder for files without a date is named `no-date/` instead of
  `senza-data/`.
- Two errors that carried their own prose inside `Metadata(String)` became
  typed variants of their own: `UnrecognisedSource` and `ConfigDirUnavailable`.
- Added `ACKNOWLEDGEMENTS.md`, recording what the project took from others: the
  edited-suffix list from GooglePhotosTakeoutHelper (Apache-2.0) and the time
  zone boundaries derived from OpenStreetMap (ODbL-1.0), plus the licences of
  every direct dependency.
- The `PRODID` of exported calendars declared `IT`; it now declares `EN`.

### Sources and archives

- Recognition of an extracted `Takeout/` folder or a `takeout-*.zip` archive,
  with the list of sections, counts and sizes.
- Reconstruction of the entire series from any single archive
  (`takeout-...-001.zip`, `-002.zip`, ...) and merging into one tree, flagging
  the numbers missing from an incomplete download.
- Zip-slip protection: paths are normalised and entries trying to escape the
  destination are discarded rather than written.

### Photos

- Reading of EXIF and JSON sidecars, including the
  `.supplemental-metadata.json` schemas, duplicates with a counter, and the
  names Google shortens to 46 characters: on a long name the suffix arrives
  truncated (`.supplemental-m.json`) or disappears entirely, leaving the media
  name itself cut short.
- The date is derived from the camera-generated filename when both EXIF and
  sidecar are missing (`IMG_20200101_120000`, `PXL_...`, screenshots, Signal).
- Rewriting into the EXIF tags of JPEG, HEIC, TIFF and WebP, without
  recompressing the image, of **everything the sidecar holds that has a home in
  the metadata**: capture date with its time zone, coordinates, description
  (`ImageDescription` and `XPComment`), recognised faces (`XPKeywords`) and the
  favourite star (`Rating` and `RatingPercent`). Only the view count and the
  Google Photos URL are left out, because metadata has nowhere to put them: the
  app lists them rather than letting the user find out.
- **Local wall-clock time instead of the universal instant.** When a photo
  carries coordinates, the time zone is derived locally and written together
  with its offset, accounting for the daylight saving rules in force that day.
  Writing the UTC instant as-is would have shifted every photo by the zone
  offset.
- Output layout of your choosing: original structure, by year, by year and
  month, or a single folder. Files without a date end up in `no-date/`
  rather than being filed under an invented month.
- Three modes: dry run, repaired copy in a separate tree (the default), and
  rewriting the originals, which requires an explicit confirmation.

### Albums

- Recognition of Google Photos albums, which the export records not as separate
  information but as folders containing a second copy of the photo.
- Year folders are told apart from real albums in any account language, derived
  from the export itself rather than from a table of translations: the year
  prefix (`Photos from`, `Foto da`, `Fotos de`) is identical across all the
  years of a given export, so it can be deduced instead of guessed. When two
  different prefixes appear the same number of times the distinction cannot be
  made, and the app says so instead of picking at random.
- A membership manifest that can be written to file. Until it exists,
  deduplication on the photo folder stays blocked: files come back from
  quarantine, album membership does not.
- Recognition of edited versions (`-edited`, `-modificato`, `-modifié`,
  `-編集済み` and nine other languages), which are not treated as duplicates.

### Sidecars set aside

- After rewriting the originals the `.json` files remain in the folder, and the
  app offers to move them elsewhere. It moves only those that are no longer the
  sole copy of anything, and it does not take the repair's own word for it:
  every file is read back to confirm that date, coordinates, description, faces
  and favourite really are there.
- The sidecars of PNG, GIF and video files stay where they are, since those
  formats have no EXIF block and the JSON is therefore the only home for their
  metadata; so do those of photos not yet repaired, and those carrying data with
  no counterpart in the tags. Every reason for staying is counted and shown.
- This is not a deletion: it writes the same ledger as quarantine, so the
  restore puts every JSON back where it was.

### Contacts and calendar

- A vCard parser handling line folding, group prefixes and escaping,
  deduplicating by email or normalised phone number, exporting standard
  vCard 3.0.
- An iCalendar parser that does not mistake alarms for events, deduplicates by
  UID and occurrence, strips `X-GOOGLE-*` properties, and exports a conformant
  `.ics`.

### Drive and cleanup

- Classification by category and detection of the `.gdoc`/`.gsheet`
  placeholders, which hold a reference rather than the data.
- Deduplication **by content**: two files with the same name and the same size
  but different content both survive.
- No mode deletes: either a clean tree is built elsewhere, or files move to
  quarantine with a ledger that puts everything back with one click.
- When a media file is removed its JSON sidecar follows it, so no orphans are
  left behind.
- The cleanup engine works on any section, not just Drive.

### Disk space

- The space needed is computed before starting, and the operation is refused if
  there is not enough, rather than filling the disk halfway through and leaving
  an output tree that looks complete.
- When space is short the app does more than say so: it offers in-place
  rewriting, which needs a few dozen megabytes no matter how large the library.
- A list of the subfolders that fit in the space left, with a button to repair
  them one at a time.
- In that list years and albums are distinguished, and folders containing files
  that exist nowhere else are flagged: those cannot be postponed without losing
  track of them.

### Interface

- A built-in guide, reachable from the header button and the Help menu.
- A first-run introduction with a "do not show again" checkbox, the only piece
  of state that outlives the session.
- An application menu with explicit labels, because in development macOS
  otherwise shows the executable name.

### Privacy

- A `deny.toml` that bans network, telemetry and updater crates, verified in CI:
  adding an HTTP client fails the build.
- A CSP with `connect-src` limited to the local IPC channel.
- The window capability reduced to `core:default` and the two system pickers.
  The frontend has no direct filesystem access.
- No clickable links anywhere in the interface: addresses, including those in
  Drive placeholders, are selectable text.
- `PRIVACY_AUDIT.md`, with the verification performed on the release bundle.

### Verification

- 71 tests, including end-to-end runs on synthetic Takeouts. EXIF repair is
  verified by reading back the tags from a real JPEG and comparing the
  coordinates after a round trip through degrees, minutes and seconds.
  Quarantine is verified by taking a snapshot of paths and contents before the
  operation and demanding that the restore reproduce it exactly.
- Five measurements excluded from CI, to be run by hand: a hundred thousand
  photo library, the real-bytes path, a large address book and calendar,
  extraction of a multi-archive series taken from disk, and repair followed by
  setting aside the sidecars on a real folder.
- `tools/generate_takeout_series.py` builds the material for the last two: fifteen
  gigabytes across eight archives, with the known quirks of the export. It
  exists because a test that generates its own data also validates its own
  assumptions, and if an assumption is wrong the test stays green anyway.
- `cargo clippy -- -D warnings`, `cargo deny check` and `npm run build` in CI.

### Fixes made during development

- Edited versions whose names are in decomposed form (NFD, the way macOS writes
  them) were truncated at the wrong point, producing names like `IMG_1-.jpg`.
  The cut point is now searched on the original string.
- Pixel names carrying milliseconds (`PXL_20200101_120000123`) were rejected by
  the date recognition.
- The search for companion files was quadratic in the number of files: on a
  hundred thousand photos it went from 411 seconds to 0.7.
- Files in a format unsupported by EXIF rewriting had stopped being copied into
  the repaired tree.
- Sidecars whose names Google had shortened were not recognised, so the photo
  ended up taking its date from its own filename: a poorer source, and one that
  carries no coordinates. On a fifteen gigabyte test series that was 87 photos
  out of 3237.
- An album called `Christmas 2024` was mistaken for a year folder, so its
  membership never reached the manifest, losing precisely the information the
  manifest exists to save.

The last two surfaced from the measurement on a multi-archive series taken from
disk, not from the synthetic tests: those were generating the material with the
same assumptions they were verifying.
