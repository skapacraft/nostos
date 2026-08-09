#!/usr/bin/env python3
# Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Genera una serie multi-archivio che imita un export di Google Foto.

Serve a dare materiale al test `estrazione_di_una_serie_reale`, che estrae una
serie presa dal disco invece di costruirsela da sé. Il senso è proprio questo:
un test che genera i propri dati verifica anche le proprie assunzioni, e se
un'assunzione è sbagliata il test resta verde lo stesso.

Va detto con chiarezza: **questo non è un Takeout prodotto da Google.**
Riproduce la struttura, la nomenclatura degli archivi, la divisione a fette e
le stranezze note dell'export, ma non può garantire di riprodurre ogni scelta
del suo scrittore zip. Per quelle serve un export vero, chiesto a Google con la
dimensione massima per archivio impostata bassa così che lo divida davvero.

Stranezze riprodotte:

- ogni archivio ripete la radice `Takeout/` e le cartelle di sezione;
- le foto di una stessa annata sono sparse su più archivi;
- i nomi dei sidecar sono accorciati a 46 caratteri, `.json` compreso, e quando
  il taglio ne fa collidere due arriva un contatore;
- un media su diciannove non ha sidecar;
- gli album sono cartelle che contengono una copia identica della foto, che è
  il modo in cui Google esporta l'appartenenza a un album;
- esistono coppie originale/modificato.

Uso:

    tools/genera_serie_takeout.py ~/Downloads/prova-multiarchivio

La dimensione si regola con le variabili d'ambiente `OTH_GB` (totale, 15 per
impostazione predefinita) e `OTH_GB_ARCHIVIO` (per singolo archivio, 1,9 come
l'opzione "2 GB" di Google). Servono altrettanti gigabyte liberi per estrarre.
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

# Un solo serbatoio di byte casuali, riusato a fette: generare quindici
# gigabyte di casualità vera costerebbe più della scrittura stessa.
POOL = os.urandom(16 * 1024 * 1024)


def corpo(indice, dimensione):
    """Byte del file: intestazione JPEG reale più riempimento distinto.

    Il marcatore serve a rendere ogni file diverso da tutti gli altri anche a
    parità di dimensione. Senza, la deduplica per contenuto li unirebbe in un
    gruppo solo e la misura non direbbe nulla sul caso reale.
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
    """Distribuzione realistica: la maggior parte fra 2,5 e 7 megabyte."""
    return int(2.5 * 1024 * 1024 + ((indice * 104729) % 4608) * 1024)


def sidecar(nome, quando, lat, lon):
    """Il JSON affiancato, nello schema che Google scrive oggi."""
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
    """Accorcia il nome fino a MAX_NOME_SIDECAR caratteri, `.json` compreso."""
    completo = f"{nome_media}.supplemental-metadata.json"
    if len(completo) <= MAX_NOME_SIDECAR:
        return completo
    return completo[: MAX_NOME_SIDECAR - len(".json")] + ".json"


def costruisci_piano():
    """Elenca i media da scrivere, prima di toccare il disco."""
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
            # Nome Pixel, con i millisecondi in coda.
            nome = f"PXL_{anno}{mese:02d}{giorno:02d}_{ora:02d}{minuto:02d}00{indice % 1000:03d}.jpg"
        elif indice % 23 == 3:
            # Nome lungo, che costringe Google ad accorciare il sidecar.
            nome = f"Foto scattata durante la gita del {giorno:02d}-{mese:02d}-{anno}.jpg"
        else:
            nome = f"IMG_{indice:05d}.JPG"

        cartella = f"Takeout/Google Foto/Foto da {anno}"
        # Un media su diciannove arriva senza sidecar, come capita davvero.
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

        # Copia identica in un album: è così che Google esporta l'appartenenza.
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
    """Contatti e calendario, con i duplicati che la deduplica deve trovare."""
    schede = []
    for i in range(400):
        schede.append(
            f"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Contatto {i}\r\n"
            f"N:Cognome{i};Nome{i};;;\r\nEMAIL:contatto{i}@example.com\r\n"
            f"TEL:+39 320 {1_000_000 + i}\r\nEND:VCARD\r\n"
        )
        if i % 5 == 0:
            # Stessa email con altre maiuscole: va riconosciuta come duplicato.
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
            eventi.append(evento)  # stesso UID due volte
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
    """Scrive gli archivi uno dopo l'altro, chiudendoli alla dimensione voluta."""

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
        # Le foto non si comprimono: Google le mette nello zip così come sono,
        # e i JSON invece li comprime. Riprodurre entrambi i casi tiene in
        # esercizio tutti e due i percorsi di lettura.
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
        # L'accorciamento può far collidere due sidecar: Google in quel caso
        # aggiunge un contatore, e scrivere due volte lo stesso nome nello zip
        # lascerebbe invece una voce irraggiungibile.
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
