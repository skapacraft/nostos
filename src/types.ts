// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

/**
 * TypeScript counterpart of the Rust structs serialised by serde.
 *
 * The names follow `#[serde(rename_all = "camelCase")]`: if a field changes in
 * `src-tauri/src`, it has to be updated here too.
 */

export type SourceKind = "folder" | "archive";

/** Outcome of writing an exported file. */
export interface ExportReport {
  path: string;
  written: number;
  bytes: number;
}

/**
 * Non-blocking notice emitted by the backend.
 *
 * It arrives as a code plus the data needed to compose the sentence: the text
 * is decided in `lib/messages.ts`, not in the engine.
 */
export type Notice =
  | { code: "noSectionsFound" }
  | { code: "archiveNotExtracted" }
  | { code: "ambiguousYearFolders" }
  | { code: "unsafeArchiveEntries"; count: number }
  | { code: "placeholdersWithoutContent"; count: number }
  | { code: "photosOnlyInAlbums"; count: number }
  | { code: "photosSharedWithAlbums"; count: number }
  | { code: "readFailed"; path: string; detail: string };

/** Error that interrupted an operation, in the same shape as the notices. */
export type ErrorPayload =
  | { code: "io"; path: string; detail: string }
  | { code: "archive"; detail: string }
  | { code: "unsafeEntry"; entry: string }
  | { code: "metadata"; detail: string }
  | { code: "notFound"; path: string }
  | { code: "noSource" }
  | { code: "notEnoughSpace"; needed: number; available: number }
  | { code: "task"; detail: string }
  | { code: "poisoned" }
  | { code: "destinationInsideSource" }
  | { code: "destinationRequired" }
  | { code: "unrecognisedSource"; path: string }
  | { code: "configDirUnavailable"; detail: string };

/** Why a sidecar stayed where it was. */
export type SidecarKept =
  | "noExifContainer"
  | "unreadableExif"
  | "missingDate"
  | "missingGeo"
  | "missingDescription"
  | "missingPeople"
  | "missingFavorite"
  | "viewCountHasNoTag"
  | "photoUrlHasNoTag";

export interface KeptReason {
  reason: SidecarKept;
  count: number;
}

export type TakeoutSection =
  | "googlePhotos"
  | "contacts"
  | "drive"
  | "mail"
  | "calendar"
  | "youTube"
  | "other";

export interface SectionSummary {
  /** Category of the section: the readable name is chosen by `SECTION_LABELS`. */
  section: TakeoutSection;
  /** Name of the folder on disk, which must not be translated. */
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
  warnings: Notice[];
}

/** Phase of a long-running operation. */
export type Phase = "scanning" | "extracting" | "writing" | "done";

/** Progress emitted by the backend on the `takeout://progress` event. */
export interface Progress {
  phase: Phase;
  done: number;
  total: number;
  errors: number;
  current: string | null;
}

/**
 * The only preferences kept between one run and the next.
 *
 * Every field added here is data outliving the session and has to be declared
 * in PRIVACY_AUDIT.md.
 */
export interface Preferences {
  hideWelcome: boolean;
}

/**
 * Space arithmetic, to choose the mode before starting.
 *
 * On a large library the question is not whether the operation works, but
 * whether it fits: the copy duplicates everything, in-place does not.
 */
export interface FolderSize {
  name: string;
  path: string;
  bytes: number;
  fileCount: number;
  /** True if the copy of this folder alone fits. */
  fits: boolean;
  /** Year folder: these are the slices worth repairing. */
  isYear: boolean;
  /** Album: mostly copies of photos already present in the year folders. */
  isAlbum: boolean;
  /**
   * Photos of this folder that exist in no year folder.
   *
   * It is the one number that, ignored, loses something: skipping an album to
   * save space makes sense only while it stays at zero.
   */
  uniqueHere: number;
}

export interface SpaceEstimate {
  sourceBytes: number;
  availableBytes: number;
  neededForCopy: number;
  copyFits: boolean;
  /** Extra space for in-place rewriting: tens of megabytes, always. */
  neededInPlace: number;
  /** Slices to divide the work into when the whole library does not fit. */
  subfolders: FolderSize[];
}

/** Application metadata, read from Cargo.toml at compile time. */
export interface AppInfo {
  name: string;
  version: string;
  author: string;
  homepage: string;
  repository: string;
  license: string;
}

/** The guarantees declared by the backend, one per verifiable point. */
export type PrivacyNote =
  | "noHttpCrates"
  | "restrictiveCsp"
  | "noUpdaterNoOpener"
  | "dataStaysLocal";

export interface PrivacyReport {
  networkCalls: boolean;
  telemetry: boolean;
  crashReporting: boolean;
  autoUpdater: boolean;
  externalLinks: boolean;
  notes: PrivacyNote[];
}

// --- Archives ------------------------------------------------------------

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
  /** Archives processed, in numbering order. */
  archives: string[];
  filesWritten: number;
  dirsCreated: number;
  bytesWritten: number;
  skipped: string[];
  /** Paths present in more than one archive of the series. */
  collisions: string[];
}

/** The series of archives making up a single export. */
export interface ArchiveSeries {
  prefix: string;
  archives: string[];
  /** Missing numbers: they indicate an incomplete download. */
  missing: number[];
  totalCompressedBytes: number;
}

// --- Photos --------------------------------------------------------------

/** In decreasing order of reliability. */
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
  /** Date derived from the filename, the last resort. */
  dateFromFilename: number;
  totalBytes: number;
  /** Complete count of the unreadable files. */
  unreadableCount: number;
  /** Sample of the problems, truncated for the interface. */
  unreadable: string[];
  sample: MediaRecord[];
}

/**
 * How to treat the originals.
 *
 * `dryRun` touches nothing, `copyToOutput` writes into a separate tree and is
 * the default, `inPlace` rewrites the originals and has to be chosen by hand.
 */
export type WriteMode = "dryRun" | "copyToOutput" | "inPlace";

/**
 * Layout of the output tree. It applies only with `copyToOutput`.
 *
 * It matters less than it seems: once the date is written into the EXIF, photo
 * managers sort on that and ignore the folders. It serves those who keep their
 * photos in plain folders, without a program indexing them.
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
  /** Sidecars kept beside the files whose EXIF was not written. */
  sidecarsCopied: number;
  failures: string[];
}

// --- Contacts ------------------------------------------------------------

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
  warnings: Notice[];
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
  warnings: Notice[];
}

// --- Calendar ------------------------------------------------------------

export interface CalendarEvent {
  uid: string | null;
  summary: string | null;
  location: string | null;
  description: string | null;
  /** Start in the raw iCalendar form, e.g. `20200101T120000Z`. */
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
  /** Proprietary properties removed during cleanup. */
  droppedProperties: number;
  warnings: Notice[];
  sample: CalendarEvent[];
}

// --- Drive cleanup -------------------------------------------------------

/**
 * How to treat the files being removed.
 *
 * A deleting mode is missing on purpose: both working modes produce something
 * that can be undone.
 */
export type CleanMode = "dryRun" | "copyToOutput" | "quarantine";

export type QuarantineReason = "junk" | "duplicate";

export interface CleanOptions {
  mode: CleanMode;
  destination: string | null;
  removeJunk: boolean;
  removeDuplicates: boolean;
  /** Takes along the sidecars of the media removed. */
  moveCompanions: boolean;
}

/** Group of files with identical content, verified by hash. */
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
  /** `duplicateGroups` is truncated for the interface: the counts above are not. */
  reclaimableBytes: number;
  hashedBytes: number;
  duplicateGroups: ContentDuplicateGroup[];
  junkSample: string[];
  warnings: Notice[];
}

export interface CleanReport {
  mode: CleanMode | null;
  destination: string | null;
  filesKept: number;
  duplicatesHandled: number;
  junkHandled: number;
  companionsHandled: number;
  bytesReclaimed: number;
  /** Quarantine ledger, the only way to undo the operation. */
  manifest: string | null;
  failures: string[];
}

export interface RestoreReport {
  restored: number;
  skippedExisting: number;
  failures: string[];
}

export interface SidecarSweepReport {
  destination: string;
  moved: number;
  bytesMoved: number;
  /** Sidecars left where they were because still the sole copy of something. */
  kept: number;
  /** Why they stayed, with how many times each reason occurs. */
  keptReasons: KeptReason[];
  keptSample: string[];
  /** Ledger of the move, the only way to undo it. */
  manifest: string | null;
  failures: string[];
}

// --- Google Photos albums ------------------------------------------------

export interface Album {
  name: string;
  path: string;
  /** How many files it holds, complete count. */
  fileCount: number;
  /** Sample of the names, truncated for the interface. */
  files: string[];
}

/** A photo present both in a year folder and in one or more albums. */
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
  /** Complete count of the memberships. */
  membershipCount: number;
  /** Sample of the memberships, truncated for the interface. */
  memberships: AlbumMembership[];
  /** Complete count of the edited versions. */
  editedCount: number;
  editedPairs: EditedPair[];
  /** Photos present only in an album: removing them would make them vanish. */
  albumOnly: number;
  warnings: Notice[];
}
