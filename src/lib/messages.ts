// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

/**
 * The text of everything the backend describes as a code.
 *
 * The engine composes no sentences: it declares what happened and with which
 * numbers, and the language is decided here. This is the single place to
 * translate for notices, errors, section and category labels: were a sentence
 * ever to reappear inside the Rust, it would move back out of the reach of any
 * translation.
 *
 * The maps are declared as `Record`s over the complete union of codes rather
 * than as free objects: adding a variant in Rust without the matching text
 * fails the build here instead of showing a raw code on screen. Adding a
 * locale gets the same guarantee, one level up: `Record<Locale, ...>` means a
 * new locale with a missing table fails the build too.
 */

import { formatBytes, formatCount } from "./format";
import { locale } from "./locale";
import type {
  ErrorPayload,
  FileCategory,
  Notice,
  PrivacyNote,
  SidecarKept,
  TakeoutSection,
} from "../types";

const SECTION_LABELS_BY_LOCALE: Record<
  Locale,
  Record<TakeoutSection, string>
> = {
  en: {
    googlePhotos: "Google Photos",
    contacts: "Contacts",
    drive: "Drive",
    mail: "Mail",
    calendar: "Calendar",
    youTube: "YouTube",
    other: "Other",
  },
  it: {
    googlePhotos: "Google Foto",
    contacts: "Contatti",
    drive: "Drive",
    mail: "Mail",
    calendar: "Calendario",
    youTube: "YouTube",
    other: "Altro",
  },
};

const CATEGORY_LABELS_BY_LOCALE: Record<
  Locale,
  Record<FileCategory, string>
> = {
  en: {
    document: "Documents",
    spreadsheet: "Spreadsheets",
    presentation: "Presentations",
    pdf: "PDF",
    image: "Images",
    video: "Video",
    audio: "Audio",
    archive: "Archives",
    code: "Code",
    placeholder: "Placeholders with no content",
    other: "Other",
  },
  it: {
    document: "Documenti",
    spreadsheet: "Fogli di calcolo",
    presentation: "Presentazioni",
    pdf: "PDF",
    image: "Immagini",
    video: "Video",
    audio: "Audio",
    archive: "Archivi",
    code: "Codice",
    placeholder: "Segnaposto senza contenuto",
    other: "Altro",
  },
};

const SIDECAR_KEPT_LABELS_BY_LOCALE: Record<
  Locale,
  Record<SidecarKept, string>
> = {
  en: {
    noExifContainer:
      "the format has no EXIF block to write into (PNG, GIF, video)",
    unreadableExif: "the file has no readable EXIF block",
    missingDate:
      "the capture date does not appear to be written into the file",
    missingGeo: "the coordinates do not appear to be written into the file",
    missingDescription:
      "the description does not appear to be written into the file",
    missingPeople:
      "the recognised faces do not appear to be written into the file",
    missingFavorite:
      "the favourite mark does not appear to be written into the file",
    viewCountHasNoTag: "the view count has no tag to live in",
    photoUrlHasNoTag: "the Google Photos address has no tag to live in",
  },
  it: {
    noExifContainer:
      "il formato non ha un blocco EXIF dove scrivere (PNG, GIF, video)",
    unreadableExif: "il file non ha un blocco EXIF leggibile",
    missingDate: "la data di scatto non risulta scritta nel file",
    missingGeo: "le coordinate non risultano scritte nel file",
    missingDescription: "la descrizione non risulta scritta nel file",
    missingPeople: "i volti riconosciuti non risultano scritti nel file",
    missingFavorite:
      "il contrassegno di preferito non risulta scritto nel file",
    viewCountHasNoTag:
      "il conteggio delle visualizzazioni non ha un tag dove stare",
    photoUrlHasNoTag: "l'indirizzo su Google Foto non ha un tag dove stare",
  },
};

const PRIVACY_NOTES_BY_LOCALE: Record<Locale, Record<PrivacyNote, string>> = {
  en: {
    noHttpCrates: "No HTTP crate anywhere in the dependency graph.",
    restrictiveCsp:
      "Restrictive CSP: connect-src is limited to the local IPC channel.",
    noUpdaterNoOpener: "No updater and no URL opening plugin is registered.",
    dataStaysLocal: "Your data stays in the folders you choose, and in memory.",
  },
  it: {
    noHttpCrates: "Nessuna crate HTTP nel grafo delle dipendenze.",
    restrictiveCsp:
      "CSP restrittiva: connect-src è limitato al canale IPC locale.",
    noUpdaterNoOpener:
      "Nessun updater e nessun plugin di apertura URL registrato.",
    dataStaysLocal:
      "I dati restano nei percorsi scelti dall'utente e in memoria.",
  },
};

type Locale = "en" | "it";

/** Readable name of an export section. */
export const SECTION_LABELS: Record<TakeoutSection, string> =
  SECTION_LABELS_BY_LOCALE[locale()];

/** Readable name of a file category. */
export const CATEGORY_LABELS: Record<FileCategory, string> =
  CATEGORY_LABELS_BY_LOCALE[locale()];

/** Why a sidecar stayed where it was instead of being set aside. */
export const SIDECAR_KEPT_LABELS: Record<SidecarKept, string> =
  SIDECAR_KEPT_LABELS_BY_LOCALE[locale()];

/** The privacy guarantees, as the guide shows them. */
export const PRIVACY_NOTES: Record<PrivacyNote, string> =
  PRIVACY_NOTES_BY_LOCALE[locale()];

/** Text of a non-blocking notice. */
export function noticeText(notice: Notice): string {
  if (locale() === "it") {
    switch (notice.code) {
      case "noSectionsFound":
        return "Nessuna sezione Takeout riconosciuta: verifica di aver selezionato la cartella che contiene Google Foto, Drive o Contatti.";
      case "archiveNotExtracted":
        return "Archivio non estratto: estrailo per esaminare foto, contatti e Drive.";
      case "unsafeArchiveEntries":
        return `${formatCount(notice.count)} voci dell'archivio hanno percorsi non sicuri e verranno saltate in estrazione.`;
      case "placeholdersWithoutContent":
        return `${formatCount(notice.count)} file sono segnaposto Google senza contenuto: l'export porta un riferimento a qualcosa online, non il dato stesso.`;
      case "photosOnlyInAlbums":
        return `${formatCount(notice.count)} foto compaiono solo dentro un album e in nessuna cartella per anno: rimuoverle dagli album le farebbe sparire del tutto.`;
      case "photosSharedWithAlbums":
        return `${formatCount(notice.count)} foto sono duplicate tra cartelle per anno e album. Salva il manifest prima di deduplicare, altrimenti l'appartenenza agli album va persa.`;
      case "ambiguousYearFolders":
        return "Più cartelle finiscono con un anno senza condividere un prefisso, quindi non è possibile dire quali siano annate e quali album con l'anno nel nome. Sono state trattate tutte come annate, il che significa che il manifest potrebbe non registrare un'appartenenza a un album.";
      case "readFailed":
        return `Non è stato possibile leggere ${notice.path}.`;
    }
  }
  switch (notice.code) {
    case "noSectionsFound":
      return "No Takeout section recognised: check that you picked the folder containing Google Photos, Drive or Contacts.";
    case "archiveNotExtracted":
      return "The archive has not been extracted: extract it to examine photos, contacts and Drive.";
    case "unsafeArchiveEntries":
      return `${formatCount(notice.count)} entries in the archive have unsafe paths and will be skipped during extraction.`;
    case "placeholdersWithoutContent":
      return `${formatCount(notice.count)} files are Google placeholders with no content: the export carries a reference to something online, not the data itself.`;
    case "photosOnlyInAlbums":
      return `${formatCount(notice.count)} photos appear only inside an album and in no year folder: removing them from the albums would make them disappear entirely.`;
    case "photosSharedWithAlbums":
      return `${formatCount(notice.count)} photos are duplicated between year folders and albums. Save the manifest before deduplicating, or album membership is lost.`;
    case "ambiguousYearFolders":
      return "Several folders end with a year without sharing a prefix, so there is no telling which are years and which are albums with a year in the name. All of them were treated as years, which means the manifest may not record an album membership.";
    case "readFailed":
      return `Could not read ${notice.path}.`;
  }
}

/**
 * Technical detail of a notice, when there is one.
 *
 * It comes from the operating system or a library, in the language they chose:
 * it belongs beside the sentence as a detail, not in its place.
 */
export function noticeDetail(notice: Notice): string | null {
  return notice.code === "readFailed" ? notice.detail : null;
}

/** Text of an error that interrupted an operation. */
export function errorText(payload: ErrorPayload): string {
  if (locale() === "it") {
    switch (payload.code) {
      case "io":
        return `Impossibile leggere o scrivere ${payload.path}.`;
      case "archive":
        return "L'archivio non è valido, oppure è danneggiato.";
      case "unsafeEntry":
        return `L'archivio contiene una voce che tenta di scrivere fuori dalla destinazione: ${payload.entry}.`;
      case "metadata":
        return "I metadati non sono interpretabili.";
      case "notFound":
        return `Percorso non trovato: ${payload.path}.`;
      case "noSource":
        return "Nessuna sorgente Takeout caricata.";
      case "notEnoughSpace":
        return `Spazio insufficiente sulla destinazione: servono ${formatBytes(payload.needed)} e ne restano ${formatBytes(payload.available)}.`;
      case "task":
        return "L'elaborazione in background si è interrotta.";
      case "poisoned":
        return "Stato interno non coerente: riavvia l'applicazione.";
      case "destinationInsideSource":
        return "La destinazione non può stare dentro la cartella di origine.";
      case "destinationRequired":
        return "Questa modalità richiede di scegliere una destinazione.";
      case "unrecognisedSource":
        return `${payload.path} non è né una cartella Takeout né un archivio takeout-*.zip.`;
      case "configDirUnavailable":
        return "La cartella di configurazione del sistema non è raggiungibile.";
    }
  }
  switch (payload.code) {
    case "io":
      return `Could not read or write ${payload.path}.`;
    case "archive":
      return "The archive is not valid, or it is damaged.";
    case "unsafeEntry":
      return `The archive contains an entry that tries to write outside the destination: ${payload.entry}.`;
    case "metadata":
      return "The metadata cannot be interpreted.";
    case "notFound":
      return `Path not found: ${payload.path}.`;
    case "noSource":
      return "No Takeout source is loaded.";
    case "notEnoughSpace":
      return `Not enough room on the destination: ${formatBytes(payload.needed)} is needed and ${formatBytes(payload.available)} is left.`;
    case "task":
      return "The background work stopped.";
    case "poisoned":
      return "Internal state is inconsistent: restart the application.";
    case "destinationInsideSource":
      return "The destination cannot sit inside the source folder.";
    case "destinationRequired":
      return "This mode needs a destination to be chosen.";
    case "unrecognisedSource":
      return `${payload.path} is neither a Takeout folder nor a takeout-*.zip archive.`;
    case "configDirUnavailable":
      return "The system configuration folder cannot be reached.";
  }
}

/** Technical detail of an error, when there is one. */
export function errorDetail(payload: ErrorPayload): string | null {
  switch (payload.code) {
    case "io":
    case "archive":
    case "metadata":
    case "task":
    case "configDirUnavailable":
      return payload.detail;
    default:
      return null;
  }
}
