# Open Takeout Hub

A local-first desktop application for processing Google Takeout exports.
It analyses photos, contacts and Drive without a single byte leaving your
computer.

## Why

A Takeout is a raw, hard-to-navigate archive: photos lose their EXIF and carry
the date in a JSON sidecar, contacts arrive as vCards full of duplicates, Drive
contains placeholders that hold no content. The online tools that solve these
problems ask you to upload the entire export to a third-party server, which is
precisely the data you were trying to take back into your own hands.

Open Takeout Hub does the same work locally.

## Stack

| Layer    | Technology                                |
| -------- | ----------------------------------------- |
| Shell    | Tauri 2                                   |
| Backend  | Rust stable, 2021 edition                 |
| Frontend | React 19, TypeScript 5.8, Vite 7          |
| Styling  | Tailwind CSS 4 (Vite plugin, no PostCSS)  |

## Privacy guarantees

These are not good intentions. They are constraints you can verify in the code.

1. **No network crates.** There is no HTTP client anywhere in the Rust
   dependency graph. Check it with `cargo tree`.
2. **No telemetry and no crash reporter.** No installation identifier is ever
   generated or stored.
3. **No auto-updater.** `createUpdaterArtifacts` is disabled and the updater
   plugin is not installed.
4. **Restrictive CSP.** `connect-src` is limited to the local IPC channel, so
   even a `fetch` added to the frontend by mistake would be blocked by the
   webview. See `app.security.csp` in `src-tauri/tauri.conf.json`.
5. **No link opener.** The `opener` plugin that ships with the template has been
   removed: URLs found inside Drive placeholders are shown as text and are not
   clickable.
6. **Minimal permissions.** The window capability grants `core:default`,
   `dialog:allow-open` and `dialog:allow-save`, that is the two system pickers
   and nothing else. The frontend has no direct filesystem access: every read
   and every write goes through an explicit Rust command, and the path always
   comes from a dialog the user opened.
7. **No implicit persistence.** Results live in memory for the duration of the
   session. Every write to disk originates from an explicit action: extracting
   an archive, repairing photos, exporting contacts or calendar, quarantining
   Drive files.
8. **Free space is checked before writing.** A repaired copy duplicates the
   library: a sixty gigabyte export needs another sixty free. The operation is
   refused before it starts when there is not enough room, rather than filling
   the disk halfway through and leaving an output tree that looks complete.
   When it does not fit the app does more than say so: it offers in-place
   rewriting, which needs a few dozen megabytes no matter how large the
   library, and lists the subfolders that do fit in the space left, with a
   button to repair them one at a time.
9. **Nothing is ever deleted.** No function in this application removes files.
   Drive cleanup either builds an alternative tree or moves files to quarantine,
   writing a ledger that lets you undo everything. The same holds for sidecars
   left behind after a repair: they are moved, never removed, and only after the
   file has been read back to confirm their content really is inside it.

The `privacy_report` command exposes this declaration to the UI, which shows it
behind the "Offline" badge in the header.

## Layout

```
src-tauri/
  deny.toml          the network ban, in executable form
  fixtures/          real JPEG used by the EXIF writing tests
  src/
    lib.rs           composition root: state, commands, events, plugins
    app_state.rs     shared state, errors, progress, sections, notices
    zip_handler.rs   archive series, merging, zip-slip protection
    exif_parser.rs   EXIF and sidecars, reconciliation and rewriting
    contacts.rs      vCard 3.0 parser, deduplication, export
    calendar.rs      iCalendar parser, cleanup, export
    albums.rs        albums, year folders, edited versions
    drive.rs         classification, placeholders, dedup and quarantine
                     (the cleanup engine works on any folder)

tools/
  genera_serie_takeout.py   builds a multi-archive test series

src/
  App.tsx              session orchestration
  types.ts             TypeScript counterpart of the serde structs
  lib/api.ts           the only point of contact with the backend (IPC)
  lib/format.ts        shared formatting helpers
  lib/messages.ts      every string the backend describes as a code
  components/
    Dropzone.tsx       drop area built on native Tauri events
    SourcePanel.tsx    source summary and section list
    Reports.tsx        photo, contacts, calendar and Drive report views
    PhotoFixer.tsx     metadata repair with mode selection
    ProgressBar.tsx    progress fed by backend events
    ExportButton.tsx   saving exported files
    AlbumPanel.tsx     albums, membership manifest, edited versions
    FolderCleaner.tsx  folder cleanup, with preview and undo
    Notices.tsx        non-blocking warnings emitted by the backend
    Help.tsx           in-app guide and licence information
    SidecarSweep.tsx   sets aside JSONs whose content is now in the files
    Welcome.tsx        first-run introduction, dismissable for good
    Stat.tsx           numeric tile
```

## What it does today

- **Source**: recognises an extracted `Takeout/` folder or a `takeout-*.zip`
  archive, and lists the sections with counts and sizes.
- **Archives**: given any single archive it reconstructs the whole series
  (`takeout-...-001.zip`, `-002.zip`, ...) and merges it into one tree, flagging
  the missing numbers of an incomplete download. Paths are normalised and
  entries trying to escape the destination are rejected.
- **Albums**: Google does not export albums as separate information but as
  folders containing a second copy of the photo. The app recognises them and
  tells year folders from real albums in any language, without a table of
  translations: it derives from the export itself the prefix that export uses
  for years (`Photos from`, `Foto da`, `Fotos de`), so an album called
  `Christmas 2024` stays an album. It then writes a membership manifest. Until
  that manifest exists, deduplication on the photo folder stays blocked: the
  files would come back from quarantine, the membership would not.
- **Photos**: reads EXIF and JSON sidecars (including the
  `.supplemental-metadata.json` schemas, the duplicates with a counter, and the
  names Google shortens to 46 characters, where the suffix arrives truncated or
  disappears entirely), and when both are missing it derives the date from the
  camera-generated filename (`IMG_20200101_120000`, `PXL_...`, screenshots,
  Signal). It **writes into the EXIF tags** of JPEG, HEIC, TIFF and WebP,
  without recompressing the image, everything the sidecar holds that has a home
  in the metadata: date, coordinates, description (`ImageDescription`),
  recognised faces (`XPKeywords`) and the favourite star (`Rating`). Only the
  view count and the Google Photos URL are left out, because metadata has
  nowhere to put them, and the app says so rather than letting you find out.
  When a photo carries coordinates, the app derives the local time zone and
  writes the correct wall-clock time with its offset, accounting for daylight
  saving: `DateTimeOriginal` is the time the clock showed on the spot, not
  universal time, and writing a UTC instant into it would shift every photo.
  The repaired copy can keep the original structure or be reorganised by year,
  by year and month, or into a single folder; files without a date end up in
  `no-date/` rather than being filed under an invented month. It recognises
  edited versions (`-edited`, `-modificato`, `-modifié`, `-編集済み` and others)
  and does not treat them as duplicates. Three modes: dry run, repaired copy in
  a separate tree (the default), and rewriting the originals, which requires an
  explicit confirmation.
- **Contacts**: a vCard parser that handles line folding, group prefixes and
  escaping, deduplicates by email or normalised phone number, and exports a
  standard vCard 3.0.
- **Calendar**: an iCalendar parser that does not mistake alarms for events,
  deduplicates by UID and occurrence, strips `X-GOOGLE-*` properties and exports
  a conformant `.ics` with correct line folding.
- **Drive**: classification by category, detection of the `.gdoc`/`.gsheet`
  placeholders that hold no data, and cleanup with deduplication **by content**:
  two files with the same name and size but different content both survive. No
  mode deletes: either a clean tree is built elsewhere, or files move to
  quarantine with a ledger that puts everything back with one click. When a
  media file is removed its JSON sidecar follows it, so no orphans are left
  behind.
- **Sidecars set aside once a repair is done**: after rewriting the originals
  the `.json` files remain in the folder, and the app offers to move them. It
  moves only those that are no longer the sole copy of anything, and it does not
  take the repair's own word for it: every file is read back to verify the data
  really is there. The sidecars of PNG, GIF and video files stay, as do those of
  photos that were not repaired and those carrying data with no home in the
  tags. This is not a deletion: it writes the same ledger as quarantine and
  undoes with one click.
- **Cleanup works on any section**, not just Drive: it is available on Google
  Photos too, where exports often contain the same shot several times because it
  belongs to several albums.
- **Built-in guide**, reachable from the header button and the Help menu, with a
  first-run introduction for anyone opening the app for the first time.

## Quality

Every change goes through four checks, all of them run in CI:

| Check | What it guarantees |
| --- | --- |
| `cargo deny check` | no network, telemetry or updater crates |
| `cargo clippy -- -D warnings` | zero warnings |
| `cargo test` | 71 tests, including end-to-end runs on synthetic Takeouts |
| `npm run build` | types aligned with the serde structs |

There are also five measurements excluded from CI, meant to be run by hand. The
first works on a large library, because it generates tens of thousands of files:

```bash
FOTO=100000 cargo test --release --manifest-path src-tauri/Cargo.toml \
  misura_su_libreria_grande -- --ignored --nocapture
```

You do not need a hundred gigabyte export to find scaling problems: what puts
this code under strain is the number of files, not the number of bytes. A
hundred thousand synthetic photos take a hundred and fifty megabytes and are a
harsher test than a real library of the same size.

The real-bytes path has a measurement of its own, which writes a few gigabytes:

```bash
GB=2 /usr/bin/time -l cargo test --release --manifest-path src-tauri/Cargo.toml \
  misura_su_file_grandi -- --ignored --nocapture
```

A third one covers contacts and calendar, which have the opposite profile: very
few files, but large, and read into memory in one piece.

```bash
CONTATTI=20000 EVENTI=50000 cargo test --release --manifest-path src-tauri/Cargo.toml \
  misura_su_rubrica_grande -- --ignored --nocapture
```

The bytes measurement checks deduplication and repair throughput, and above all
that a file past the rewrite threshold is skipped but still copied. On two and a
half gigabytes of media, allocated memory stays around a hundred megabytes
(`peak memory footprint`; `maximum resident set size` includes the file page
cache and does not measure what the program allocates).

The fourth extracts a multi-archive series taken from disk rather than building
one for itself. The difference is not cosmetic: a test that generates its own
data also validates its own assumptions, and if an assumption is wrong the test
stays green anyway. The material is prepared with the script in `tools/`:

```bash
tools/genera_serie_takeout.py ~/Downloads/prova-multiarchivio

SERIE=~/Downloads/prova-multiarchivio USCITA=~/Downloads/prova-estratta \
  cargo test --release --manifest-path src-tauri/Cargo.toml \
  estrazione_di_una_serie_reale -- --ignored --nocapture
```

On fifteen gigabytes split across eight archives, the way Google produces them
with the "2 GB" option: the series is recognised from a single archive in
0.1 ms, 6330 files are extracted in 40 seconds at 383 MB/s with no collisions,
3237 photos are scanned in 1.4 seconds, and allocated memory peaks at 107 MB.

The fifth repairs a real folder and then sets aside the applied sidecars,
working on a copy so the original stays untouched:

```bash
CARTELLA="~/Downloads/prova-estratta/Takeout/Google Foto/Foto da 2019" \
  cargo test --release --manifest-path src-tauri/Cargo.toml \
  ripara_e_mette_da_parte_i_sidecar -- --ignored --nocapture
```

The script does not produce a Google Takeout: it reproduces the structure, the
naming, the splitting into slices and the known quirks of the export, but not
the choices of Google's own zip writer. Those can only be verified against a
real export, requested from Google with the maximum archive size set low. This
is worth stating because that measurement has already found two genuine defects
the synthetic tests could not see: sidecars whose names Google had shortened
were not being recognised, and an album called `Christmas 2024` was ending up
among the year folders.

The tests do not stop at "it did not blow up". EXIF repair is verified by
reading back the tags written into a real JPEG and comparing the coordinates
after a round trip through degrees, minutes and seconds. Quarantine is verified
by taking a snapshot of paths and contents before the operation and demanding
that the restore reproduce it exactly.

## What it does not do yet

- **PNG**: deliberately excluded from EXIF rewriting. See
  [PRIVACY_AUDIT.md](PRIVACY_AUDIT.md), section 8.
- **Video**: the metadata lives in container atoms, not in EXIF. For those, only
  the modification date is aligned.
- **Mail and YouTube**: recognised in the summary, with no analyser. A Gmail
  `.mbox` needs an on-disk index, which is a project of its own.
- Analysing sections directly inside the archive: a ZIP has to be extracted
  first.

## Development

Prerequisites: Node 20 or newer, a stable Rust toolchain, and the Xcode Command
Line Tools on macOS.

```bash
npm install
npm run tauri dev
```

Other useful commands:

```bash
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo deny --manifest-path src-tauri/Cargo.toml check
```

## Distribution

Tauri bundles do not cross-compile between platforms: macOS will not produce
`.msi` or `.deb`. A local build produces the package for the system it runs on:

```bash
npm run tauri build
```

On macOS the final step, the one that lays out the `.dmg` window, uses
AppleScript to talk to the Finder. It has to run from a terminal authorised to
send Apple Events: from a process that is not, it fails with error -1743 after
having produced the `.app`, which remains usable.

For every platform there is `.github/workflows/release.yml`, which on a `v*` tag
builds macOS (universal), Linux (`.deb` and `.AppImage`) and Windows (`.msi`,
`.nsis` and the portable executable) on their respective runners. The Linux
runner is pinned to Ubuntu 22.04: building on a newer distribution produces
packages that will not start on LTS ones.

## Language

The interface is currently Italian only. English is the intended default, with
Italian, German, French and Spanish to follow.

The groundwork is done: the Rust backend never composes a sentence. It reports
codes and numbers, and the wording is chosen on the side that displays it, so
everything a user reads lives in `src/lib/messages.ts` and in the components.
That separation is enforced by the type system rather than by convention:
adding a variant in Rust without the matching text fails the frontend build.

Nothing is fetched at runtime. Translations are bundled, because reaching out
to a translation service would contradict the one promise this application
makes.

## Attribution

Time zone boundaries come from [OpenStreetMap](https://www.openstreetmap.org/copyright),
distributed by the `tzf-dist` package under the
[Open Database License](https://opendatacommons.org/licenses/odbl/) (ODbL-1.0).
The data is bundled with the application and consulted locally.

## Author

Built by **SkapaCraft** ([skapacraft.com](https://skapacraft.com)).

## Licence

Copyright (C) 2026 SkapaCraft. GPL-3.0-or-later, see [LICENSE](LICENSE).

The choice is deliberate: the GPL prevents anyone from taking this code, adding
telemetry to it and redistributing it as a proprietary binary. For an
application whose only promise is "it does not watch you", a permissive licence
would be a contradiction.
