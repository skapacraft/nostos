// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared application state and common types.
//!
//! All state lives in memory for the duration of the session: no implicit
//! persistence, no configuration file written behind your back, no installation
//! identifier generated.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The single error propagated to the Tauri commands.
#[derive(Debug, Error)]
pub enum TakeoutError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid archive: {0}")]
    Archive(String),

    #[error("unsafe archive entry (path traversal): {0}")]
    UnsafeEntry(String),

    #[error("unreadable metadata: {0}")]
    Metadata(String),

    #[error("path not found: {0}")]
    NotFound(PathBuf),

    #[error("no Takeout source loaded")]
    NoSource,

    #[error(
        "not enough space on the destination: {} needed, {} left",
        crate::app_state::formatta_byte(*needed),
        crate::app_state::formatta_byte(*available)
    )]
    NotEnoughSpace { needed: u64, available: u64 },

    #[error("background processing interrupted: {0}")]
    Task(String),

    #[error("internal state corrupted: poisoned lock")]
    Poisoned,

    #[error("the destination cannot sit inside the source folder")]
    DestinationInsideSource,

    #[error("this mode requires a destination")]
    DestinationRequired,
}

impl TakeoutError {
    /// Builds an I/O error keeping the path that caused it.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// The form in which an error crosses the IPC channel.
///
/// An error reaching the interface as a finished sentence is a sentence no
/// translation can ever get to. It therefore travels as a code plus the data
/// needed to compose the message on the other side.
///
/// `detail` carries the message of whoever detected the fault, operating system
/// or library: it is not ours and it is not translated, so it belongs next to
/// the sentence as a technical detail, not in its place.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum ErrorPayload {
    Io { path: String, detail: String },
    Archive { detail: String },
    UnsafeEntry { entry: String },
    Metadata { detail: String },
    NotFound { path: String },
    NoSource,
    NotEnoughSpace { needed: u64, available: u64 },
    Task { detail: String },
    Poisoned,
    DestinationInsideSource,
    DestinationRequired,
}

impl TakeoutError {
    /// Turns the error into the form that crosses the IPC channel.
    pub fn payload(&self) -> ErrorPayload {
        match self {
            Self::Io { path, source } => ErrorPayload::Io {
                path: path.display().to_string(),
                detail: source.to_string(),
            },
            Self::Archive(detail) => ErrorPayload::Archive {
                detail: detail.clone(),
            },
            Self::UnsafeEntry(entry) => ErrorPayload::UnsafeEntry {
                entry: entry.clone(),
            },
            Self::Metadata(detail) => ErrorPayload::Metadata {
                detail: detail.clone(),
            },
            Self::NotFound(path) => ErrorPayload::NotFound {
                path: path.display().to_string(),
            },
            Self::NoSource => ErrorPayload::NoSource,
            Self::NotEnoughSpace { needed, available } => ErrorPayload::NotEnoughSpace {
                needed: *needed,
                available: *available,
            },
            Self::Task(detail) => ErrorPayload::Task {
                detail: detail.clone(),
            },
            Self::Poisoned => ErrorPayload::Poisoned,
            Self::DestinationInsideSource => ErrorPayload::DestinationInsideSource,
            Self::DestinationRequired => ErrorPayload::DestinationRequired,
        }
    }
}

impl Serialize for TakeoutError {
    // `Result` in this module is the crate alias, so the full form of the
    // standard one is needed here.
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.payload().serialize(serializer)
    }
}

pub type Result<T> = std::result::Result<T, TakeoutError>;

/// A non-blocking notice meant for the user.
///
/// The backend does not compose the sentence: it declares what happened and
/// with which numbers, and whoever displays it decides how to say it. Without
/// that separation a translated interface would stay peppered with phrases
/// written here, and every new language would mean going back into the engine.
///
/// The one exception is [`Notice::ReadFailed`]: the detail comes from the
/// operating system or a library, in whatever language they chose. Translating
/// it is not in our power, so it travels as-is and belongs on screen as a
/// technical detail, not as a sentence addressed to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum Notice {
    /// The chosen folder holds no recognisable Takeout sections.
    NoSectionsFound,
    /// Entries with an unsafe path, which extraction will ignore.
    UnsafeArchiveEntries { count: usize },
    /// Google placeholders: the export carries the reference, not the content.
    PlaceholdersWithoutContent { count: usize },
    /// Photos present only inside an album and in no year folder.
    PhotosOnlyInAlbums { count: usize },
    /// Photos appearing both in a year folder and in an album.
    PhotosSharedWithAlbums { count: usize },
    /// Years cannot be told apart from albums with a year in their name.
    AmbiguousYearFolders,
    /// The source is an archive: extract it before analysing its sections.
    ArchiveNotExtracted,
    /// Read failure, with the original message of whoever detected it.
    ReadFailed { path: String, detail: String },
}

impl Notice {
    /// A read-failure notice built from any error.
    pub fn read_failed(path: impl std::fmt::Display, detail: impl std::fmt::Display) -> Self {
        Self::ReadFailed {
            path: path.to_string(),
            detail: detail.to_string(),
        }
    }
}

/// Development diagnostics, printed on the `tauri dev` terminal.
///
/// Deliberately not a file logger: writing logs to disk would contradict
/// section 6 of `PRIVACY_AUDIT.md`, which promises to leave no trace of the
/// session. Here the output goes to stderr and the whole block is compiled out
/// of release builds, so in the distributed binary these lines do not exist.
macro_rules! trace_dev {
    ($($arg:tt)*) => {{
        #[cfg(debug_assertions)]
        {
            eprintln!("[oth] {}", format_args!($($arg)*));
        }
    }};
}

pub(crate) use trace_dev;

/// Identifying data of the application, shown in the guide.
///
/// The values come from the variables Cargo exposes at compile time: they are
/// the same ones as `Cargo.toml`, so they cannot drift from the metadata of the
/// distributed package.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub author: String,
    pub homepage: String,
    pub repository: String,
    pub license: String,
}

impl Default for AppInfo {
    fn default() -> Self {
        Self {
            name: "Open Takeout Hub".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            author: env!("CARGO_PKG_AUTHORS").to_string(),
            homepage: env!("CARGO_PKG_HOMEPAGE").to_string(),
            repository: env!("CARGO_PKG_REPOSITORY").to_string(),
            license: env!("CARGO_PKG_LICENSE").to_string(),
        }
    }
}

/// The only preferences the application keeps between runs.
///
/// The file holds this field alone, and it is the single exception to the rule
/// of writing nothing implicitly. The struct stays deliberately minimal: every
/// field added here is one more piece of data outliving the session, and has to
/// be declared in `PRIVACY_AUDIT.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    /// The user asked not to see the first-run introduction again.
    pub hide_welcome: bool,
}

/// Outcome of writing an exported file.
///
/// Shared by contacts and calendar: both produce a single standard file ready
/// to be imported elsewhere.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: PathBuf,
    /// Items written to the file.
    pub written: usize,
    pub bytes: u64,
}

/// Phase of a long-running operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Scanning,
    Extracting,
    Writing,
    Done,
}

/// Progress sent to the UI during a long-running operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: Phase,
    pub done: usize,
    pub total: usize,
    pub errors: usize,
    /// Name of the file being processed, without the full path.
    pub current: Option<String>,
}

impl Progress {
    pub fn new(phase: Phase, done: usize, total: usize, errors: usize) -> Self {
        Self {
            phase,
            done,
            total,
            errors,
            current: None,
        }
    }

    pub fn with_current(mut self, current: impl Into<String>) -> Self {
        self.current = Some(current.into());
        self
    }
}

/// The progress channel handed to the domain modules.
///
/// The modules know nothing of Tauri: they receive a closure and cannot tell
/// whether an event towards the webview, a counter in a test or nothing at all
/// sits behind it. `Send` and `Sync` are needed because photo processing runs
/// on several threads.
pub type ProgressSink<'a> = &'a (dyn Fn(Progress) + Send + Sync);

/// A sink that discards everything, for callers that show no progress.
pub fn no_progress(_: Progress) {}

/// Helpers shared by the tests of the various modules.
#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};

    /// A temporary folder that deletes itself when the test ends.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "open-takeout-hub-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("creazione cartella temporanea");
            Self(path)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Writes a file, creating the intermediate folders.
    pub(crate) fn write_file(path: &Path, content: &str) {
        write_bytes(path, content.as_bytes());
    }

    /// Binary variant, for the real image fixtures.
    pub(crate) fn write_bytes(path: &Path, content: &[u8]) {
        std::fs::create_dir_all(path.parent().expect("path with a parent"))
            .expect("creazione cartelle");
        std::fs::write(path, content).expect("scrittura file");
    }

    /// A valid 8x8 pixel JPEG, to exercise real EXIF writing.
    ///
    /// A genuine container is required: `little_exif` rightly refuses a file with
    /// the wrong signature, so a fake text JPEG would only verify the error
    /// handling.
    pub(crate) const MINIMAL_JPEG: &[u8] = include_bytes!("../fixtures/minimal.jpg");
}

/// The kind of source the user selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    /// An already extracted `Takeout/` folder.
    Folder,
    /// An unextracted `takeout-*.zip` archive.
    Archive,
}

/// A Google Takeout section recognised inside the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TakeoutSection {
    GooglePhotos,
    Contacts,
    Drive,
    Mail,
    Calendar,
    YouTube,
    Other,
}

impl TakeoutSection {
    /// Derives the section from the top-level folder name inside `Takeout/`.
    ///
    /// The names are localised into the account language, so the match covers the
    /// most common Italian and English variants.
    pub fn from_dir_name(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "google foto" | "google photos" => Self::GooglePhotos,
            "contatti" | "contacts" => Self::Contacts,
            "drive" | "google drive" => Self::Drive,
            "mail" | "posta" => Self::Mail,
            "calendar" | "calendario" => Self::Calendar,
            "youtube e youtube music" | "youtube and youtube music" | "youtube" => Self::YouTube,
            _ => Self::Other,
        }
    }
}

/// Summary of a single section found in the source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionSummary {
    /// The section as a category, not as a label: the readable name is chosen by
    /// whoever displays it, in the language they are using.
    pub section: TakeoutSection,
    /// The folder name exactly as it is on disk, which must not be translated.
    pub dir_name: String,
    pub path: PathBuf,
    pub file_count: usize,
    pub total_bytes: u64,
}

/// Summary of the loaded source, returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub root: PathBuf,
    pub display_name: String,
    pub kind: SourceKind,
    pub sections: Vec<SectionSummary>,
    pub file_count: usize,
    pub total_bytes: u64,
    /// Non-blocking notices that surfaced during the scan.
    pub warnings: Vec<Notice>,
}

/// The guarantees the application declares, one per verifiable point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrivacyNote {
    /// There is no HTTP crate in the dependency graph.
    NoHttpCrates,
    /// The CSP limits `connect-src` to the local IPC channel alone.
    RestrictiveCsp,
    /// No auto-updater and no plugin for opening URLs.
    NoUpdaterNoOpener,
    /// Data stays in the paths chosen by the user, and in memory.
    DataStaysLocal,
}

/// The source currently loaded in the session.
///
/// `root` is redundant with `summary.root` but remains the authoritative field
/// for the commands that work on the path without touching the summary.
#[derive(Debug, Clone)]
pub struct LoadedSource {
    pub root: PathBuf,
    pub summary: SourceSummary,
}

#[derive(Debug, Default)]
struct StateInner {
    source: Option<LoadedSource>,
}

/// Shared state registered with `tauri::Builder::manage`.
#[derive(Debug, Default)]
pub struct AppState {
    inner: Mutex<StateInner>,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the loaded source, replacing any previous one.
    pub fn set_source(&self, source: LoadedSource) -> Result<()> {
        let mut guard = self.inner.lock().map_err(|_| TakeoutError::Poisoned)?;
        guard.source = Some(source);
        Ok(())
    }

    /// Returns the summary of the current source.
    pub fn summary(&self) -> Result<SourceSummary> {
        let guard = self.inner.lock().map_err(|_| TakeoutError::Poisoned)?;
        guard
            .source
            .as_ref()
            .map(|s| s.summary.clone())
            .ok_or(TakeoutError::NoSource)
    }

    /// Root of the current source.
    pub fn root(&self) -> Result<PathBuf> {
        let guard = self.inner.lock().map_err(|_| TakeoutError::Poisoned)?;
        guard
            .source
            .as_ref()
            .map(|s| s.root.clone())
            .ok_or(TakeoutError::NoSource)
    }

    /// Empties the state: used by the "Close source" command.
    pub fn clear(&self) -> Result<()> {
        let mut guard = self.inner.lock().map_err(|_| TakeoutError::Poisoned)?;
        guard.source = None;
        Ok(())
    }
}

/// Explicit declaration of the privacy profile, exposed to the UI.
///
/// The values are compiled constants: were anyone to introduce a network
/// dependency one day, this block would have to be updated by hand and the
/// change would stay visible in the diff.
///
/// The notes are codes and not sentences, like everything else that ends up on
/// screen: a guarantee written in one language would be readable in one
/// language.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyReport {
    pub network_calls: bool,
    pub telemetry: bool,
    pub crash_reporting: bool,
    pub auto_updater: bool,
    pub external_links: bool,
    pub notes: Vec<PrivacyNote>,
}

impl Default for PrivacyReport {
    fn default() -> Self {
        Self {
            network_calls: false,
            telemetry: false,
            crash_reporting: false,
            auto_updater: false,
            external_links: false,
            notes: vec![
                PrivacyNote::NoHttpCrates,
                PrivacyNote::RestrictiveCsp,
                PrivacyNote::NoUpdaterNoOpener,
                PrivacyNote::DataStaysLocal,
            ],
        }
    }
}

/// Human-readable size, for the error messages.
pub(crate) fn formatta_byte(bytes: u64) -> String {
    const UNITA: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut valore = bytes as f64;
    let mut unita = 0;
    while valore >= 1000.0 && unita < UNITA.len() - 1 {
        valore /= 1000.0;
        unita += 1;
    }
    if unita == 0 {
        format!("{bytes} B")
    } else {
        format!("{valore:.1} {}", UNITA[unita])
    }
}

/// A subfolder of the source, with its weight.
///
/// It exists to work in slices when the whole library does not fit: repair one
/// folder at a time, move the result elsewhere, move on to the next.
/// successiva.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSize {
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub file_count: usize,
    /// True if the copy of this folder alone fits in the space left.
    pub fits: bool,
    /// True if this is a year folder: those are the slices worth repairing,
    /// because they hold nearly everything.
    pub is_year: bool,
    /// True if this is an album, that is mostly copies of photos already present
    /// elsewhere.
    pub is_album: bool,
    /// How many photos of this folder exist in no year folder at all.
    ///
    /// It is the one number that, ignored, loses something: skipping an album
    /// to save space makes sense only while this stays at zero.
    pub unique_here: usize,
}

/// Space arithmetic, to decide before starting.
///
/// It answers the question anyone with a large library asks: does it fit? And
/// if it does not, what can I do?
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceEstimate {
    /// How much the source library weighs.
    pub source_bytes: u64,
    /// How much room is left on the destination volume.
    pub available_bytes: u64,
    /// How much the copy would need, safety margin included.
    pub needed_for_copy: u64,
    /// True if the repaired copy fits.
    pub copy_fits: bool,
    /// Extra space required by in-place rewriting.
    ///
    /// It is one temporary file per thread, so it stays in the order of tens of
    /// megabytes whatever the size of the library: it is the practicable route
    /// when the copy does not fit.
    pub needed_in_place: u64,
    /// Top-level subfolders, from the heaviest to the lightest.
    ///
    /// When the whole library does not fit, these are the slices to divide the
    /// work into.
    pub subfolders: Vec<FolderSize>,
}

/// Safety margin required on top of the bytes to write.
///
/// Filling a disk to the last byte is never a good idea: the system needs room
/// for its own temporary files, and on APFS snapshots can hold on to blocks
/// that look free.
const MARGINE_DISCO: f64 = 1.10;

/// Refuses the operation when the destination has not enough room.
///
/// It exists because the repaired copy duplicates the entire library: a sixty
/// gigabyte export needs another sixty. Without this check the disk would fill
/// halfway through, leaving an output tree that looks complete and is not, and
/// the user would find out only by counting files.
pub fn require_free_space(destination: &Path, needed: u64) -> Result<()> {
    // Space is measured on the nearest existing folder: the destination may not
    // have been created yet.
    let mut probe = destination;
    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent,
            None => return Ok(()),
        }
    }

    let available = match fs4::available_space(probe) {
        Ok(bytes) => bytes,
        // If the filesystem cannot answer we do not block the work: better to try and
        // fail on a single file than to refuse for no reason.
        Err(_) => return Ok(()),
    };

    let required = (needed as f64 * MARGINE_DISCO) as u64;
    if available >= required {
        return Ok(());
    }

    Err(TakeoutError::NotEnoughSpace {
        needed: required,
        available,
    })
}

/// Checks that a path received from the frontend really exists.
pub fn require_existing(path: &Path) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(TakeoutError::NotFound(path.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn riconosce_le_sezioni_localizzate() {
        assert_eq!(
            TakeoutSection::from_dir_name("Google Foto"),
            TakeoutSection::GooglePhotos
        );
        assert_eq!(
            TakeoutSection::from_dir_name("google photos"),
            TakeoutSection::GooglePhotos
        );
        assert_eq!(TakeoutSection::from_dir_name("Keep"), TakeoutSection::Other);
    }

    #[test]
    fn lo_stato_parte_vuoto_e_si_svuota() {
        let state = AppState::new();
        assert!(matches!(state.summary(), Err(TakeoutError::NoSource)));
        state
            .clear()
            .expect("clearing an empty state must not fail");
    }
}
