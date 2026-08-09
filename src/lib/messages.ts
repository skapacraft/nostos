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
 * fails the build here instead of showing a raw code on screen.
 */

import { formatBytes, formatCount } from "./format";
import type {
  ErrorPayload,
  FileCategory,
  Notice,
  PrivacyNote,
  SidecarKept,
  TakeoutSection,
} from "../types";

/** Readable name of an export section. */
export const SECTION_LABELS: Record<TakeoutSection, string> = {
  googlePhotos: "Google Foto",
  contacts: "Contatti",
  drive: "Drive",
  mail: "Mail",
  calendar: "Calendario",
  youTube: "YouTube",
  other: "Altro",
};

/** Readable name of a file category. */
export const CATEGORY_LABELS: Record<FileCategory, string> = {
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
};

/** Why a sidecar stayed where it was instead of being set aside. */
export const SIDECAR_KEPT_LABELS: Record<SidecarKept, string> = {
  noExifContainer:
    "il formato non ha un blocco EXIF dove scrivere (PNG, GIF, video)",
  unreadableExif: "il file non ha un blocco EXIF leggibile",
  missingDate: "la data di scatto non risulta scritta nel file",
  missingGeo: "le coordinate non risultano scritte nel file",
  missingDescription: "la descrizione non risulta scritta nel file",
  missingPeople: "i volti riconosciuti non risultano scritti nel file",
  missingFavorite: "il contrassegno di preferito non risulta scritto nel file",
  viewCountHasNoTag: "il conteggio delle visualizzazioni non ha un tag dove stare",
  photoUrlHasNoTag: "l'indirizzo su Google Foto non ha un tag dove stare",
};

/** The privacy guarantees, as the guide shows them. */
export const PRIVACY_NOTES: Record<PrivacyNote, string> = {
  noHttpCrates: "Nessuna crate HTTP nel grafo delle dipendenze.",
  restrictiveCsp: "CSP restrittiva: connect-src limitato al canale IPC locale.",
  noUpdaterNoOpener:
    "Nessun updater e nessun plugin di apertura URL registrato.",
  dataStaysLocal:
    "I dati restano nei percorsi scelti dall'utente e in memoria.",
};

/** Text of a non-blocking notice. */
export function noticeText(notice: Notice): string {
  switch (notice.code) {
    case "noSectionsFound":
      return "Nessuna sezione Takeout riconosciuta: verifica di aver selezionato la cartella che contiene Google Foto, Drive o Contatti.";
    case "archiveNotExtracted":
      return "Archivio non estratto: estrailo per analizzare foto, contatti e Drive.";
    case "unsafeArchiveEntries":
      return `${formatCount(notice.count)} voci dell'archivio hanno percorsi non sicuri e verranno ignorate in estrazione.`;
    case "placeholdersWithoutContent":
      return `${formatCount(notice.count)} file sono segnaposto Google senza contenuto: l'export non include i dati, solo un riferimento online.`;
    case "photosOnlyInAlbums":
      return `${formatCount(notice.count)} foto compaiono solo dentro un album e in nessuna cartella per anno: rimuoverle dagli album le farebbe sparire del tutto.`;
    case "photosSharedWithAlbums":
      return `${formatCount(notice.count)} foto sono duplicate tra cartelle per anno e album. Esporta il manifest prima di deduplicare, altrimenti l'appartenenza agli album va persa.`;
    case "ambiguousYearFolders":
      return "Più cartelle finiscono con un anno senza condividere un prefisso: non è possibile dire quali siano annate e quali album con l'anno nel nome. Sono state trattate tutte come annate, quindi il manifest potrebbe non registrare l'appartenenza a un album.";
    case "readFailed":
      return `Non è stato possibile leggere ${notice.path}.`;
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
  switch (payload.code) {
    case "io":
      return `Errore di lettura o scrittura su ${payload.path}.`;
    case "archive":
      return "L'archivio non è valido o è danneggiato.";
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
      return "Stato interno non consistente: riavvia l'applicazione.";
    case "destinationInsideSource":
      return "La destinazione non può stare dentro la cartella di origine.";
    case "destinationRequired":
      return "Questa modalità richiede di scegliere una destinazione.";
  }
}

/** Technical detail of an error, when there is one. */
export function errorDetail(payload: ErrorPayload): string | null {
  switch (payload.code) {
    case "io":
    case "archive":
    case "metadata":
    case "task":
      return payload.detail;
    default:
      return null;
  }
}
