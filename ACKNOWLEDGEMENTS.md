# Acknowledgements

What this project took from others, and from whom.

Two of the entries below are licence obligations rather than courtesies: the
ODbL requires attribution for the time zone data, and Apache-2.0 asks that
notices be retained. The rest is here because knowing where something came from
is part of knowing whether to trust it.

## Data taken from other projects

### GooglePhotosTakeoutHelper

[TheLastGimbus/GooglePhotosTakeoutHelper](https://github.com/TheLastGimbus/GooglePhotosTakeoutHelper),
Apache License 2.0.

The list of suffixes Google appends to edited versions of a photo, in
`src-tauri/src/albums.rs`, comes from that project. Google documents it
nowhere; the list was assembled from real exports in a dozen account languages,
and no single export could produce it.

Apache-2.0 permits the incorporation into a GPL-3.0 work and asks that the
notice be retained, which is what this entry does. No code was copied: that
project is written in Dart, this one in Rust.

## Data bundled with the application

### Time zone boundaries

The polygons used to derive a photo's time zone from its coordinates come from
[timezone-boundary-builder](https://github.com/evansiroky/timezone-boundary-builder),
which builds them from [OpenStreetMap](https://www.openstreetmap.org/copyright)
data. They reach this application through the
[tzf-rs](https://github.com/ringsaturn/tzf-rs) crate.

**The data is licensed under the [Open Database License](https://opendatacommons.org/licenses/odbl/)
(ODbL-1.0)**, which requires this attribution. It is bundled with the
application and consulted locally: no lookup leaves the machine.

## Direct dependencies

### Rust

| Crate | Licence | What it does here |
| --- | --- | --- |
| `tauri`, `tauri-plugin-dialog` | Apache-2.0 OR MIT | application shell and the system file pickers |
| `serde`, `serde_json` | MIT OR Apache-2.0 | serialisation across the IPC channel |
| `zip` | MIT | reading the Takeout archives |
| `kamadak-exif` | BSD-2-Clause | reading EXIF tags |
| `little_exif` | MIT OR Apache-2.0 | writing EXIF tags |
| `walkdir` | Unlicense OR MIT | walking the folder trees |
| `thiserror` | MIT OR Apache-2.0 | error definitions |
| `filetime` | MIT OR Apache-2.0 | aligning file modification dates |
| `chrono`, `chrono-tz` | MIT OR Apache-2.0 | dates and time zone rules |
| `rayon` | MIT OR Apache-2.0 | parallel scanning and rewriting |
| `blake3` | CC0-1.0 OR Apache-2.0 | content hashing for deduplication |
| `unicode-normalization` | MIT OR Apache-2.0 | NFC/NFD comparison of filenames |
| `tzf-rs` | MIT | time zone lookup from coordinates |
| `fs4` | MIT OR Apache-2.0 | free space on the destination volume |

### Frontend

| Package | Licence |
| --- | --- |
| `react`, `react-dom` | MIT |
| `@tauri-apps/api`, `@tauri-apps/plugin-dialog` | MIT OR Apache-2.0 |
| `tailwindcss`, `@tailwindcss/vite` | MIT |
| `vite`, `@vitejs/plugin-react` | MIT |
| `typescript` | Apache-2.0 |

The complete transitive list, with the licence of every crate, is checked in CI
by `cargo deny check licenses` and can be reproduced with:

```bash
cargo deny --manifest-path src-tauri/Cargo.toml check licenses
```

## Licence of this project

Open Takeout Hub is GPL-3.0-or-later, see [LICENSE](LICENSE). Every dependency
above is under a licence compatible with it.
