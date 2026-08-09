#!/usr/bin/env python3
# Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generates a multi-archive series imitating a Google Photos export.

It provides material for the `estrazione_di_una_serie_reale` test, which
extracts a series taken from disk instead of building one for itself. That is
precisely the point: a test that generates its own data also validates its own
assumptions, and if an assumption is wrong the test stays green anyway.

Let it be said plainly: **this is not a Takeout produced by Google.** It
reproduces the structure, the archive naming, the splitting into slices and the
known quirks of the export, but it cannot guarantee reproducing every choice of
Google's own zip writer. For those you need a real export, requested from
Google with the maximum archive size set low so that it actually splits it.

Quirks reproduced:

- every archive repeats the `Takeout/` root and the section folders;
- the photos of a single year are spread across several archives;
- sidecar names are shortened to 46 characters, `.json` included, and when the
  cut makes two of them collide a counter appears;
- one media file in nineteen has no sidecar;
- albums are folders holding an identical copy of the photo, which is how
  Google exports album membership;
- original/edited pairs exist.

Usage:

    tools/genera_serie_takeout.py ~/Downloads/prova-multiarchivio

The size is controlled by the environment variables `OTH_GB` (total, 15 by
default) and `OTH_GB_PER_ARCHIVE` (per archive, 1.9 like Google's "2 GB" option).
Extracting needs as many gigabytes free again.
"""

import datetime
import json
import os
import sys
import zipfile

GB = 1024**3
TARGET_BYTES = int(float(os.environ.get("OTH_GB", "15")) * GB)
BYTES_PER_ARCHIVE = int(float(os.environ.get("OTH_GB_PER_ARCHIVE", "1.9")) * GB)
MAX_SIDECAR_NAME = 46

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(REPO_ROOT, "src-tauri", "fixtures", "minimal.jpg")

YEARS = [2019, 2020, 2021, 2022, 2023, 2024, 2025, 2026]
ALBUM = [
    "Vacanze in Sicilia",
    "Compleanno di Giulia",
    "Matrimonio Anna e Luca",
    "Escursioni 2024",
]
PLACES = [
    (45.4642, 9.1900),
    (41.9028, 12.4964),
    (37.5079, 15.0830),
    (45.4408, 12.3155),
    (43.7696, 11.2558),
    (40.8518, 14.2681),
]

with open(FIXTURE, "rb") as source:
    JPEG_HEAD = source.read()

# A single reservoir of random bytes, reused in slices: generating fifteen
# gigabytes of true randomness would cost more than the writing itself.
POOL = os.urandom(16 * 1024 * 1024)


def body_bytes(index, size):
    """File bytes: a real JPEG header plus distinct padding.

    The marker makes every file different from all the others even at equal
    size. Without it, deduplication by content would merge them into a single
    group and the measurement would say nothing about the real case.
    """
    marker = f"OTH-{index:07d}-".encode() * 4
    remainder = size - len(JPEG_HEAD) - len(marker)
    start = (index * 7919) % (len(POOL) - 1024)
    chunks, taken = [], 0
    while taken < remainder:
        slice = POOL[start : start + min(remainder - taken, len(POOL) - start)]
        if not slice:
            start = 0
            continue
        chunks.append(slice)
        taken += len(slice)
        start = 0
    return JPEG_HEAD + marker + b"".join(chunks)[:remainder]


def size_for(index):
    """Realistic distribution: most of them between 2.5 and 7 megabytes."""
    return int(2.5 * 1024 * 1024 + ((index * 104729) % 4608) * 1024)


def sidecar(name, when, lat, lon):
    """The sidecar JSON, in the schema Google writes today."""
    readable = datetime.datetime.fromtimestamp(
        when, datetime.timezone.utc
    ).strftime("%d %b %Y, %H:%M:%S UTC")
    return json.dumps(
        {
            "title": name,
            "description": "",
            "photoTakenTime": {"timestamp": str(when), "formatted": readable},
            "creationTime": {"timestamp": str(when + 3600), "formatted": readable},
            "geoData": {
                "latitude": lat,
                "longitude": lon,
                "altitude": 120.0,
                "latitudeSpan": 0.0,
                "longitudeSpan": 0.0,
            },
            "geoDataExif": {"latitude": lat, "longitude": lon, "altitude": 120.0},
            "googlePhotosOrigin": {"mobileUpload": {"deviceType": "IOS_PHONE"}},
        },
        ensure_ascii=False,
        indent=2,
    ).encode()


def sidecar_name(nome_media):
    """Shortens the name to MAX_SIDECAR_NAME characters, `.json` included."""
    completo = f"{nome_media}.supplemental-metadata.json"
    if len(completo) <= MAX_SIDECAR_NAME:
        return completo
    return completo[: MAX_SIDECAR_NAME - len(".json")] + ".json"


def build_plan():
    """Lists the media to write, before touching the disk."""
    plan, total, index = [], 0, 0
    while total < TARGET_BYTES:
        year = YEARS[index % len(YEARS)]
        month = (index // len(YEARS)) % 12 + 1
        day = (index % 28) + 1
        hour = 9 + index % 12
        minute = (index * 7) % 60
        when = int(
            datetime.datetime(
                year, month, day, hour, minute, tzinfo=datetime.timezone.utc
            ).timestamp()
        )
        lat, lon = PLACES[index % len(PLACES)]
        size = size_for(index)

        if index % 37 == 5:
            # Pixel name, with the milliseconds at the end.
            name = f"PXL_{year}{month:02d}{day:02d}_{hour:02d}{minute:02d}00{index % 1000:03d}.jpg"
        elif index % 23 == 3:
            # Long name, forcing Google to shorten the sidecar.
            name = f"Foto scattata durante la gita del {day:02d}-{month:02d}-{year}.jpg"
        else:
            name = f"IMG_{index:05d}.JPG"

        folder = f"Takeout/Google Foto/Foto da {year}"
        # One media file in nineteen arrives without a sidecar, as really happens.
        data = None if index % 19 == 11 else sidecar(name, when, lat, lon)
        plan.append((f"{folder}/{name}", index, size, data))
        total += size

        if index % 31 == 7:
            stem, extension = os.path.splitext(name)
            edited = f"{stem}-edited{extension}"
            plan.append(
                (
                    f"{folder}/{edited}",
                    index + 500_000,
                    size,
                    sidecar(edited, when, lat, lon),
                )
            )
            total += size

        # An identical copy in an album: that is how Google exports membership.
        if index % 8 == 0:
            album = ALBUM[(index // 8) % len(ALBUM)]
            plan.append(
                (
                    f"Takeout/Google Foto/{album}/{name}",
                    index,
                    size,
                    sidecar(name, when, lat, lon),
                )
            )
            total += size

        index += 1
    return plan, total


def other_sections():
    """Contacts and calendar, with the duplicates deduplication has to find."""
    cards = []
    for i in range(400):
        cards.append(
            f"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Contatto {i}\r\n"
            f"N:Cognome{i};Nome{i};;;\r\nEMAIL:contatto{i}@example.com\r\n"
            f"TEL:+39 320 {1_000_000 + i}\r\nEND:VCARD\r\n"
        )
        if i % 5 == 0:
            # Same email in different case: it has to be recognised as a duplicate.
            cards.append(
                f"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Contatto {i}\r\n"
                f"EMAIL:CONTATTO{i}@EXAMPLE.COM\r\nEND:VCARD\r\n"
            )

    events = ["BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Google Inc//EN\r\n"]
    for i in range(300):
        event = (
            f"BEGIN:VEVENT\r\nUID:event-{i}@google.com\r\n"
            f"DTSTART:2026{(i % 12) + 1:02d}{(i % 28) + 1:02d}T090000Z\r\n"
            f"DTEND:2026{(i % 12) + 1:02d}{(i % 28) + 1:02d}T100000Z\r\n"
            f"SUMMARY:Evento {i}\r\nX-GOOGLE-CONFERENCE:meet\r\n"
            "BEGIN:VALARM\r\nACTION:DISPLAY\r\nSUMMARY:Promemoria\r\nEND:VALARM\r\n"
            "END:VEVENT\r\n"
        )
        events.append(event)
        if i % 7 == 0:
            events.append(event)  # same UID twice
    events.append("END:VCALENDAR\r\n")

    return [
        (
            "Takeout/archive_browser.html",
            b"<html><body><h1>Takeout</h1></body></html>",
        ),
        ("Takeout/Contatti/Tutti i contatti.vcf", "".join(cards).encode()),
        ("Takeout/Calendar/Personale.ics", "".join(events).encode()),
    ]


class Series:
    """Writes the archives one after another, closing them at the wanted size."""

    def __init__(self, destination):
        self.destination = destination
        self.number = 0
        self.zf = None
        self.folders = set()
        self.names = set()
        self.entries = 0
        self.written_bytes = 0
        self.total_entries = 0
        self.total_bytes = 0

    def open_next(self):
        self.number += 1
        name = f"takeout-20260806T120000Z-{self.number:03d}.zip"
        path = os.path.join(self.destination, name)
        # Photos do not compress: Google puts them in the zip as they are, and
        # compresses the JSONs instead. Reproducing both cases keeps both
        # reading paths exercised.
        self.zf = zipfile.ZipFile(path, "w", zipfile.ZIP_STORED, allowZip64=True)
        self.folders = set()
        self.entries = 0
        self.written_bytes = 0

    def close_current(self):
        if self.zf is None:
            return
        path = self.zf.filename
        self.zf.close()
        weight = os.path.getsize(path) / GB
        print(
            f"  {os.path.basename(path):42s} {weight:5.2f} GB  {self.entries:5d} entries",
            flush=True,
        )

    def _folders_of(self, path):
        parti = os.path.dirname(path).split("/")
        for i in range(1, len(parti) + 1):
            current = "/".join(parti[:i]) + "/"
            if current not in self.folders:
                self.zf.writestr(zipfile.ZipInfo(current), b"")
                self.folders.add(current)

    def add_text(self, path, contenuto):
        self._folders_of(path)
        self.zf.writestr(path, contenuto, zipfile.ZIP_DEFLATED)
        self.entries += 1

    def add_media(self, path, index, size, dati_sidecar):
        if self.written_bytes + size > BYTES_PER_ARCHIVE:
            self.close_current()
            self.open_next()

        self._folders_of(path)
        self.zf.writestr(path, body_bytes(index, size))
        self.entries += 1
        self.written_bytes += size
        self.total_entries += 1
        self.total_bytes += size

        if dati_sidecar is None:
            return

        folder = os.path.dirname(path)
        base = sidecar_name(os.path.basename(path))
        candidate = f"{folder}/{base}"
        # Shortening can make two sidecars collide: Google adds a counter in
        # that case, whereas writing the same name twice into the zip would
        # leave an unreachable entry.
        counter = 1
        while candidate in self.names:
            stem, _ = os.path.splitext(base)
            candidate = f"{folder}/{stem}({counter}).json"
            counter += 1
        self.names.add(candidate)
        self.zf.writestr(candidate, dati_sidecar, zipfile.ZIP_DEFLATED)
        self.entries += 1


def main():
    destination = (
        sys.argv[1]
        if len(sys.argv) > 1
        else os.path.expanduser("~/Downloads/prova-multiarchivio")
    )

    print("Planning the file list...", flush=True)
    plan, total = build_plan()
    print(f"  {len(plan)} media entries, {total / GB:.2f} GB of content", flush=True)

    os.makedirs(destination, exist_ok=True)
    for old in os.listdir(destination):
        if old.endswith(".zip"):
            os.remove(os.path.join(destination, old))

    series = Series(destination)
    series.open_next()
    for path, contenuto in other_sections():
        series.add_text(path, contenuto)

    print("Writing the archives...", flush=True)
    for path, index, size, data in plan:
        series.add_media(path, index, size, data)
    series.close_current()

    print(
        f"\nDone: {series.number} archives, {series.total_entries} media, "
        f"{series.total_bytes / GB:.2f} GB in {destination}",
        flush=True,
    )


if __name__ == "__main__":
    main()
