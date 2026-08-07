// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

/**
 * Controparte TypeScript delle struct Rust serializzate da serde.
 *
 * I nomi seguono `#[serde(rename_all = "camelCase")]`: se cambia un campo in
 * `src-tauri/src`, va aggiornato anche qui.
 */

export type SourceKind = "folder" | "archive";

/** Esito della scrittura di un file esportato. */
export interface ExportReport {
  path: string;
  written: number;
  bytes: number;
}

export type TakeoutSectionId =
  | "googlePhotos"
  | "contacts"
  | "drive"
  | "mail"
  | "calendar"
  | "youTube"
  | "other";

export interface SectionSummary {
  section: TakeoutSectionId;
  label: string;
  dirName: string;
  path: string;
  fileCount: number;
  totalBytes: number;
}

export interface SourceSummary {
  root: string;
  displayName: string;
  kind: SourceKind;
  sections: SectionSummary[];
  fileCount: number;
  totalBytes: number;
  warnings: string[];
}

/** Fase di un'operazione lunga. */
export type Phase = "scanning" | "extracting" | "writing" | "done";

/** Avanzamento emesso dal backend sull'evento `takeout://progress`. */
export interface Progress {
  phase: Phase;
  done: number;
  total: number;
  errors: number;
  current: string | null;
}

/**
 * Le uniche preferenze conservate tra un avvio e l'altro.
 *
 * Ogni campo aggiunto qui è un dato che sopravvive alla sessione e va
 * dichiarato in PRIVACY_AUDIT.md.
 */
export interface Preferences {
  hideWelcome: boolean;
}

/**
 * Conti sullo spazio, per scegliere la modalità prima di cominciare.
 *
 * Su una libreria grande la domanda non è se l'operazione funziona, ma se ci
 * sta: la copia duplica tutto, la riscrittura sul posto no.
 */
export interface FolderSize {
  name: string;
  path: string;
  bytes: number;
  fileCount: number;
  /** Vero se la copia di questa sola cartella ci sta. */
  fits: boolean;
}

export interface SpaceEstimate {
  sourceBytes: number;
  availableBytes: number;
  neededForCopy: number;
  copyFits: boolean;
  /** Spazio extra della riscrittura sul posto: decine di megabyte, sempre. */
  neededInPlace: number;
  /** Tranche in cui dividere il lavoro quando l'intera libreria non entra. */
  subfolders: FolderSize[];
}

/** Metadati dell'applicazione, letti da Cargo.toml a compilazione. */
export interface AppInfo {
  name: string;
  version: string;
  author: string;
  homepage: string;
  repository: string;
  license: string;
}

export interface PrivacyReport {
  networkCalls: boolean;
  telemetry: boolean;
  crashReporting: boolean;
  autoUpdater: boolean;
  externalLinks: boolean;
  notes: string[];
}

// --- Archivi -------------------------------------------------------------

export interface ArchiveEntry {
  name: string;
  isDir: boolean;
  size: number;
  compressedSize: number;
}

export interface ArchiveSummary {
  path: string;
  entryCount: number;
  fileCount: number;
  uncompressedBytes: number;
  compressedBytes: number;
  topLevel: string[];
  rejected: string[];
}

export interface ExtractReport {
  destination: string;
  /** Archivi elaborati, in ordine di numerazione. */
  archives: string[];
  filesWritten: number;
  dirsCreated: number;
  bytesWritten: number;
  skipped: string[];
  /** Percorsi presenti in più di un archivio della serie. */
  collisions: string[];
}

/** Serie di archivi che compongono un unico export. */
export interface ArchiveSeries {
  prefix: string;
  archives: string[];
  /** Numeri mancanti: indica un download incompleto. */
  missing: number[];
  totalCompressedBytes: number;
}

// --- Foto ----------------------------------------------------------------

/** In ordine di affidabilità decrescente. */
export type MetadataSource = "exif" | "sidecar" | "fileName" | "missing";

export interface GeoPoint {
  latitude: number;
  longitude: number;
  altitude: number | null;
}

export interface ExifData {
  takenAt: string | null;
  cameraMake: string | null;
  cameraModel: string | null;
  geo: GeoPoint | null;
  orientation: number | null;
}

export interface SidecarData {
  path: string;
  title: string | null;
  description: string | null;
  takenAt: string | null;
  createdAt: string | null;
  geo: GeoPoint | null;
}

export interface MediaRecord {
  path: string;
  fileName: string;
  sizeBytes: number;
  exif: ExifData;
  sidecar: SidecarData | null;
  resolvedTakenAt: string | null;
  takenAtSource: MetadataSource;
  resolvedGeo: GeoPoint | null;
  geoSource: MetadataSource;
  needsRepair: boolean;
}

export interface PhotoScanReport {
  mediaCount: number;
  withSidecar: number;
  withExifDate: number;
  withGeo: number;
  needsRepair: number;
  withoutExif: number;
  /** Data dedotta dal nome del file, ultima risorsa. */
  dateFromFilename: number;
  totalBytes: number;
  /** Conteggio completo dei file illeggibili. */
  unreadableCount: number;
  /** Campione dei problemi, troncato per l'interfaccia. */
  unreadable: string[];
  sample: MediaRecord[];
}

/**
 * Come trattare gli originali.
 *
 * `dryRun` non tocca nulla, `copyToOutput` scrive in un albero separato ed è il
 * valore predefinito, `inPlace` riscrive gli originali e va scelto a mano.
 */
export type WriteMode = "dryRun" | "copyToOutput" | "inPlace";

/**
 * Disposizione dell'albero di uscita. Vale solo con `copyToOutput`.
 *
 * Conta meno di quanto sembri: una volta scritta la data nell'EXIF, i gestori
 * di foto ordinano su quella e ignorano le cartelle. Serve a chi tiene le
 * foto in cartelle semplici, senza un programma che le indicizzi.
 */
export type OutputLayout = "preserve" | "byYear" | "byYearMonth" | "flat";

export interface WriteOptions {
  mode: WriteMode;
  layout: OutputLayout;
  outputRoot: string | null;
  writeExif: boolean;
  writeFileTimes: boolean;
}

export interface RepairReport {
  mode: WriteMode | null;
  outputRoot: string | null;
  candidates: number;
  exifWritten: number;
  fileTimesWritten: number;
  skippedUnsupported: number;
  skippedTooLarge: number;
  /** Sidecar conservati accanto ai file di cui non si è scritto l'EXIF. */
  sidecarsCopied: number;
  failures: string[];
}

// --- Contatti ------------------------------------------------------------

export interface Contact {
  displayName: string | null;
  givenName: string | null;
  familyName: string | null;
  emails: string[];
  phones: string[];
  organization: string | null;
  title: string | null;
  birthday: string | null;
  note: string | null;
}

export interface ContactsReport {
  sources: string[];
  total: number;
  unique: number;
  duplicates: number;
  withEmail: number;
  withPhone: number;
  withoutContactInfo: number;
  warnings: string[];
  sample: Contact[];
}

// --- Drive ---------------------------------------------------------------

export type FileCategory =
  | "document"
  | "spreadsheet"
  | "presentation"
  | "pdf"
  | "image"
  | "video"
  | "audio"
  | "archive"
  | "code"
  | "placeholder"
  | "other";

export interface CategoryStats {
  category: FileCategory;
  label: string;
  fileCount: number;
  totalBytes: number;
}

export interface PlaceholderFile {
  path: string;
  fileName: string;
  kind: string;
  targetUrl: string | null;
}

export interface DuplicateGroup {
  fileName: string;
  sizeBytes: number;
  paths: string[];
}

export interface LargeFile {
  path: string;
  fileName: string;
  sizeBytes: number;
}

export interface DriveReport {
  root: string;
  fileCount: number;
  dirCount: number;
  totalBytes: number;
  categories: CategoryStats[];
  placeholders: PlaceholderFile[];
  placeholderCount: number;
  duplicateGroups: DuplicateGroup[];
  duplicateBytes: number;
  largestFiles: LargeFile[];
  warnings: string[];
}

// --- Calendario ----------------------------------------------------------

export interface CalendarEvent {
  uid: string | null;
  summary: string | null;
  location: string | null;
  description: string | null;
  /** Inizio nella forma grezza dell'iCalendar, es. `20200101T120000Z`. */
  start: string | null;
  end: string | null;
  isRecurring: boolean;
  isAllDay: boolean;
}

export interface CalendarReport {
  sources: string[];
  total: number;
  unique: number;
  duplicates: number;
  recurring: number;
  allDay: number;
  /** Proprietà proprietarie rimosse durante la pulizia. */
  droppedProperties: number;
  warnings: string[];
  sample: CalendarEvent[];
}

// --- Pulizia Drive -------------------------------------------------------

/**
 * Come trattare i file da rimuovere.
 *
 * Manca di proposito una modalità che cancelli: entrambe quelle operative
 * producono qualcosa che si può disfare.
 */
export type CleanMode = "dryRun" | "copyToOutput" | "quarantine";

export type QuarantineReason = "junk" | "duplicate";

export interface CleanOptions {
  mode: CleanMode;
  destination: string | null;
  removeJunk: boolean;
  removeDuplicates: boolean;
  /** Porta con sé i sidecar dei media rimossi. */
  moveCompanions: boolean;
}

/** Gruppo di file con contenuto identico, verificato per hash. */
export interface ContentDuplicateGroup {
  hash: string;
  sizeBytes: number;
  kept: string;
  copies: string[];
}

export interface CleanPlan {
  root: string;
  filesScanned: number;
  filesKept: number;
  duplicateCopies: number;
  junkFiles: number;
  companionFiles: number;
  /** `duplicateGroups` è troncato per l'interfaccia: i conteggi sopra no. */
  reclaimableBytes: number;
  hashedBytes: number;
  duplicateGroups: ContentDuplicateGroup[];
  junkSample: string[];
  warnings: string[];
}

export interface CleanReport {
  mode: CleanMode | null;
  destination: string | null;
  filesKept: number;
  duplicatesHandled: number;
  junkHandled: number;
  companionsHandled: number;
  bytesReclaimed: number;
  /** Registro della quarantena, l'unico modo per annullare l'operazione. */
  manifest: string | null;
  failures: string[];
}

export interface RestoreReport {
  restored: number;
  skippedExisting: number;
  failures: string[];
}

// --- Album di Google Foto -------------------------------------------------

export interface Album {
  name: string;
  path: string;
  /** Quanti file contiene, conteggio completo. */
  fileCount: number;
  /** Campione dei nomi, troncato per l'interfaccia. */
  files: string[];
}

/** Una foto presente sia in una cartella per anno sia in uno o più album. */
export interface AlbumMembership {
  fileName: string;
  canonical: string | null;
  albums: string[];
}

export interface EditedPair {
  edited: string;
  original: string | null;
  suffix: string;
}

export interface AlbumIndex {
  root: string;
  yearFolders: string[];
  albums: Album[];
  specialFolders: string[];
  /** Conteggio completo delle appartenenze. */
  membershipCount: number;
  /** Campione delle appartenenze, troncato per l'interfaccia. */
  memberships: AlbumMembership[];
  /** Conteggio completo delle versioni modificate. */
  editedCount: number;
  editedPairs: EditedPair[];
  /** Foto presenti solo in un album: rimuoverle le farebbe sparire. */
  albumOnly: number;
  warnings: string[];
}
