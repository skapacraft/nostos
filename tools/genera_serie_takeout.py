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
default) and `OTH_GB_ARCHIVIO` (per archive, 1.9 like Google's "2 GB" option).
Extracting needs as many gigabytes free again.
"""

import datetime
import json
import os
import sys
import zipfile

GB = 1024**3
BERSAGLIO = int(float(os.environ.get("OTH_GB", "15")) * GB)
PER_ARCHIVIO = int(float(os.environ.get("OTH_GB_ARCHIVIO", "1.9")) * GB)
MAX_NOME_SIDECAR = 46

RADICE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(RADICE, "src-tauri", "fixtures", "minimal.jpg")

ANNI = [2019, 2020, 2021, 2022, 2023, 2024, 2025, 2026]
ALBUM = [
    "Vacanze in Sicilia",
    "Compleanno di Giulia",
    "Matrimonio Anna e Luca",
    "Escursioni 2024",
]
LUOGHI = [
    (45.4642, 9.1900),
    (41.9028, 12.4964),
    (37.5079, 15.0830),
    (45.4408, 12.3155),
    (43.7696, 11.2558),
    (40.8518, 14.2681),
]

with open(FIXTURE, "rb") as sorgente:
    TESTA = sorgente.read()

# A single reservoir of random bytes, reused in slices: generating fifteen
# gigabytes of true randomness would cost more than the writing itself.
POOL = os.urandom(16 * 1024 * 1024)


def corpo(indice, dimensione):
    """File bytes: a real JPEG header plus distinct padding.

    The marker makes every file different from all the others even at equal
    size. Without it, deduplication by content would merge them into a single
    group and the measurement would say nothing about the real case.
    """
    marcatore = f"OTH-{indice:07d}-".encode() * 4
    resto = dimensione - len(TESTA) - len(marcatore)
    inizio = (indice * 7919) % (len(POOL) - 1024)
    pezzi, presi = [], 0
    while presi < resto:
        fetta = POOL[inizio : inizio + min(resto - presi, len(POOL) - inizio)]
        if not fetta:
            inizio = 0
            continue
        pezzi.append(fetta)
        presi += len(fetta)
        inizio = 0
    return TESTA + marcatore + b"".join(pezzi)[:resto]


def dimensione_per(indice):
    """Realistic distribution: most of them between 2.5 and 7 megabytes."""
    return int(2.5 * 1024 * 1024 + ((indice * 104729) % 4608) * 1024)


def sidecar(nome, quando, lat, lon):
    """The sidecar JSON, in the schema Google writes today."""
    leggibile = datetime.datetime.fromtimestamp(
        quando, datetime.timezone.utc
    ).strftime("%d %b %Y, %H:%M:%S UTC")
    return json.dumps(
        {
            "title": nome,
            "description": "",
            "photoTakenTime": {"timestamp": str(quando), "formatted": leggibile},
            "creationTime": {"timestamp": str(quando + 3600), "formatted": leggibile},
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


def nome_sidecar(nome_media):
    """Shortens the name to MAX_NOME_SIDECAR characters, `.json` included."""
    completo = f"{nome_media}.supplemental-metadata.json"
    if len(completo) <= MAX_NOME_SIDECAR:
        return completo
    return completo[: MAX_NOME_SIDECAR - len(".json")] + ".json"


def costruisci_piano():
    """Lists the media to write, before touching the disk."""
    piano, totale, indice = [], 0, 0
    while totale < BERSAGLIO:
        anno = ANNI[indice % len(ANNI)]
        mese = (indice // len(ANNI)) % 12 + 1
        giorno = (indice % 28) + 1
        ora = 9 + indice % 12
        minuto = (indice * 7) % 60
        quando = int(
            datetime.datetime(
                anno, mese, giorno, ora, minuto, tzinfo=datetime.timezone.utc
            ).timestamp()
        )
        lat, lon = LUOGHI[indice % len(LUOGHI)]
        dimensione = dimensione_per(indice)

        if indice % 37 == 5:
            # Pixel name, with the milliseconds at the end.
            nome = f"PXL_{anno}{mese:02d}{giorno:02d}_{ora:02d}{minuto:02d}00{indice % 1000:03d}.jpg"
        elif indice % 23 == 3:
            # Long name, forcing Google to shorten the sidecar.
            nome = f"Foto scattata durante la gita del {giorno:02d}-{mese:02d}-{anno}.jpg"
        else:
            nome = f"IMG_{indice:05d}.JPG"

        cartella = f"Takeout/Google Foto/Foto da {anno}"
        # One media file in nineteen arrives without a sidecar, as really happens.
        dati = None if indice % 19 == 11 else sidecar(nome, quando, lat, lon)
        piano.append((f"{cartella}/{nome}", indice, dimensione, dati))
        totale += dimensione

        if indice % 31 == 7:
            radice, estensione = os.path.splitext(nome)
            modificato = f"{radice}-modificato{estensione}"
            piano.append(
                (
                    f"{cartella}/{modificato}",
                    indice + 500_000,
                    dimensione,
                    sidecar(modificato, quando, lat, lon),
                )
            )
            totale += dimensione

        # An identical copy in an album: that is how Google exports membership.
        if indice % 8 == 0:
            album = ALBUM[(indice // 8) % len(ALBUM)]
            piano.append(
                (
                    f"Takeout/Google Foto/{album}/{nome}",
                    indice,
                    dimensione,
                    sidecar(nome, quando, lat, lon),
                )
            )
            totale += dimensione

        indice += 1
    return piano, totale


def altre_sezioni():
    """Contacts and calendar, with the duplicates deduplication has to find."""
    schede = []
    for i in range(400):
        schede.append(
            f"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Contatto {i}\r\n"
            f"N:Cognome{i};Nome{i};;;\r\nEMAIL:contatto{i}@example.com\r\n"
            f"TEL:+39 320 {1_000_000 + i}\r\nEND:VCARD\r\n"
        )
        if i % 5 == 0:
            # Same email in different case: it has to be recognised as a duplicate.
            schede.append(
                f"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Contatto {i}\r\n"
                f"EMAIL:CONTATTO{i}@EXAMPLE.COM\r\nEND:VCARD\r\n"
            )

    eventi = ["BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Google Inc//EN\r\n"]
    for i in range(300):
        evento = (
            f"BEGIN:VEVENT\r\nUID:evento-{i}@google.com\r\n"
            f"DTSTART:2026{(i % 12) + 1:02d}{(i % 28) + 1:02d}T090000Z\r\n"
            f"DTEND:2026{(i % 12) + 1:02d}{(i % 28) + 1:02d}T100000Z\r\n"
            f"SUMMARY:Evento {i}\r\nX-GOOGLE-CONFERENCE:meet\r\n"
            "BEGIN:VALARM\r\nACTION:DISPLAY\r\nSUMMARY:Promemoria\r\nEND:VALARM\r\n"
            "END:VEVENT\r\n"
        )
        eventi.append(evento)
        if i % 7 == 0:
            eventi.append(evento)  # same UID twice
    eventi.append("END:VCALENDAR\r\n")

    return [
        (
            "Takeout/archive_browser.html",
            b"<html><body><h1>Takeout</h1></body></html>",
        ),
        ("Takeout/Contatti/Tutti i contatti.vcf", "".join(schede).encode()),
        ("Takeout/Calendar/Personale.ics", "".join(eventi).encode()),
    ]


class Serie:
    """Writes the archives one after another, closing them at the wanted size."""

    def __init__(self, destinazione):
        self.destinazione = destinazione
        self.numero = 0
        self.zf = None
        self.cartelle = set()
        self.nomi = set()
        self.voci = 0
        self.byte = 0
        self.voci_totali = 0
        self.byte_totali = 0

    def apri(self):
        self.numero += 1
        nome = f"takeout-20260806T120000Z-{self.numero:03d}.zip"
        percorso = os.path.join(self.destinazione, nome)
        # Photos do not compress: Google puts them in the zip as they are, and
        # compresses the JSONs instead. Reproducing both cases keeps both
        # reading paths exercised.
        self.zf = zipfile.ZipFile(percorso, "w", zipfile.ZIP_STORED, allowZip64=True)
        self.cartelle = set()
        self.voci = 0
        self.byte = 0

    def chiudi(self):
        if self.zf is None:
            return
        percorso = self.zf.filename
        self.zf.close()
        peso = os.path.getsize(percorso) / GB
        print(
            f"  {os.path.basename(percorso):42s} {peso:5.2f} GB  {self.voci:5d} voci",
            flush=True,
        )

    def _cartelle_di(self, percorso):
        parti = os.path.dirname(percorso).split("/")
        for i in range(1, len(parti) + 1):
            corrente = "/".join(parti[:i]) + "/"
            if corrente not in self.cartelle:
                self.zf.writestr(zipfile.ZipInfo(corrente), b"")
                self.cartelle.add(corrente)

    def aggiungi_testo(self, percorso, contenuto):
        self._cartelle_di(percorso)
        self.zf.writestr(percorso, contenuto, zipfile.ZIP_DEFLATED)
        self.voci += 1

    def aggiungi_media(self, percorso, indice, dimensione, dati_sidecar):
        if self.byte + dimensione > PER_ARCHIVIO:
            self.chiudi()
            self.apri()

        self._cartelle_di(percorso)
        self.zf.writestr(percorso, corpo(indice, dimensione))
        self.voci += 1
        self.byte += dimensione
        self.voci_totali += 1
        self.byte_totali += dimensione

        if dati_sidecar is None:
            return

        cartella = os.path.dirname(percorso)
        base = nome_sidecar(os.path.basename(percorso))
        candidato = f"{cartella}/{base}"
        # Shortening can make two sidecars collide: Google adds a counter in
        # that case, whereas writing the same name twice into the zip would
        # leave an unreachable entry.
        contatore = 1
        while candidato in self.nomi:
            radice, _ = os.path.splitext(base)
            candidato = f"{cartella}/{radice}({contatore}).json"
            contatore += 1
        self.nomi.add(candidato)
        self.zf.writestr(candidato, dati_sidecar, zipfile.ZIP_DEFLATED)
        self.voci += 1


def main():
    destinazione = (
        sys.argv[1]
        if len(sys.argv) > 1
        else os.path.expanduser("~/Downloads/prova-multiarchivio")
    )

    print("Preparo l'elenco dei file...", flush=True)
    piano, totale = costruisci_piano()
    print(f"  {len(piano)} voci media, {totale / GB:.2f} GB di contenuto", flush=True)

    os.makedirs(destinazione, exist_ok=True)
    for vecchio in os.listdir(destinazione):
        if vecchio.endswith(".zip"):
            os.remove(os.path.join(destinazione, vecchio))

    serie = Serie(destinazione)
    serie.apri()
    for percorso, contenuto in altre_sezioni():
        serie.aggiungi_testo(percorso, contenuto)

    print("Scrivo gli archivi...", flush=True)
    for percorso, indice, dimensione, dati in piano:
        serie.aggiungi_media(percorso, indice, dimensione, dati)
    serie.chiudi()

    print(
        f"\nFatto: {serie.numero} archivi, {serie.voci_totali} media, "
        f"{serie.byte_totali / GB:.2f} GB in {destinazione}",
        flush=True,
    )


if __name__ == "__main__":
    main()
