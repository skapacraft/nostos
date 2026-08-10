// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Analysis of the Google Drive export.
//!
//! Two things surprise anyone opening a Drive Takeout for the first time:
//!
//! 1. Native documents (Docs, Sheets, Slides) are converted into `.docx` /
//!    `.xlsx` / `.pptx` or `.pdf`, not exported in their original format.
//!
//! 2. Files merely shared with the user, and shortcuts, appear as `.gdoc` /
//!    `.gsheet` / `.gslides` placeholders: a few hundred bytes of JSON holding
//!    a URL. The content is not in the archive.
//!
//! The second point is the most common source of incomplete "backups", so the
//! module detects it and reports it explicitly.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::app_state::{trace_dev, Notice, Phase, Progress, ProgressSink, Result, TakeoutError};

/// Google placeholder extensions: files with no real content.
const STUB_EXTENSIONS: &[&str] = &[
    "gdoc", "gsheet", "gslides", "gdraw", "gform", "gsite", "gmap", "gjam", "gtable", "gscript",
    "glink", "gnote",
];

/// Operating system service files, of no value to the user.
const JUNK_NAMES: &[&str] = &[
    ".DS_Store",
    "desktop.ini",
    "Thumbs.db",
    "ehthumbs.db",
    ".localized",
    "Icon\r",
];

/// Service file prefixes: `._name` is the AppleDouble half of a file.
const JUNK_PREFIXES: &[&str] = &["._"];

/// Service folders: everything inside them is junk.
const JUNK_DIRS: &[&str] = &[
    "__MACOSX",
    ".Spotlight-V100",
    ".Trashes",
    ".fseventsd",
    ".TemporaryItems",
];

/// Name of the ledger written into the quarantine.
const MANIFEST_NAME: &str = "oth-quarantena.json";

/// Size of the read buffer used while hashing.
///
/// The hash is computed as a stream: a 10 GB file still occupies only this
/// buffer, and never enters memory whole.
const HASH_BUFFER: usize = 64 * 1024;

/// The kind of thing a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileCategory {
    Document,
    Spreadsheet,
    Presentation,
    Pdf,
    Image,
    Video,
    Audio,
    Archive,
    Code,
    /// A Google placeholder with no content.
    Placeholder,
    Other,
}

impl FileCategory {
    /// Derives the category from the extension.
    pub fn from_path(path: &Path) -> Self {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return Self::Other;
        };
        let ext = ext.to_ascii_lowercase();

        if STUB_EXTENSIONS.contains(&ext.as_str()) {
            return Self::Placeholder;
        }

        match ext.as_str() {
            "doc" | "docx" | "odt" | "rtf" | "txt" | "md" => Self::Document,
            "xls" | "xlsx" | "ods" | "csv" | "tsv" => Self::Spreadsheet,
            "ppt" | "pptx" | "odp" => Self::Presentation,
            "pdf" => Self::Pdf,
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "svg" | "bmp" | "tif"
            | "tiff" => Self::Image,
            "mp4" | "mov" | "avi" | "mkv" | "webm" | "m4v" | "3gp" => Self::Video,
            "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "opus" => Self::Audio,
            "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => Self::Archive,
            "rs" | "js" | "ts" | "tsx" | "jsx" | "py" | "java" | "c" | "h" | "cpp" | "go"
            | "rb" | "php" | "sh" | "sql" | "json" | "yaml" | "yml" | "toml" | "html" | "css" => {
                Self::Code
            }
            _ => Self::Other,
        }
    }
}

/// Aggregate per category.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryStats {
    pub category: FileCategory,
    pub file_count: usize,
    pub total_bytes: u64,
}

/// A placeholder found, with the reference pointing at the online content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceholderFile {
    pub path: PathBuf,
    pub file_name: String,
    pub kind: String,
    /// URL held in the placeholder, shown as text and never opened by the app.
    pub target_url: Option<String>,
}

/// A group of files sharing a name and a size.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub file_name: String,
    pub size_bytes: u64,
    pub paths: Vec<PathBuf>,
}

/// The heaviest files found in the export.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub path: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
}

/// Outcome of analysing a Drive folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveReport {
    pub root: PathBuf,
    pub file_count: usize,
    pub dir_count: usize,
    pub total_bytes: u64,
    pub categories: Vec<CategoryStats>,
    pub placeholders: Vec<PlaceholderFile>,
    pub placeholder_count: usize,
    pub duplicate_groups: Vec<DuplicateGroup>,
    /// Bytes reclaimable by removing the duplicates.
    pub duplicate_bytes: u64,
    pub largest_files: Vec<LargeFile>,
    pub warnings: Vec<Notice>,
}

/// Reads the URL held in a Google placeholder.
///
/// The file is a small JSON: if it cannot be parsed we return `None` rather
/// than failing the whole scan.
fn read_placeholder_url(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Walks the Drive folder and produces the report.
///
/// `max_items` caps the length of the lists returned to the UI: the counts stay
/// complete.
pub fn scan_directory(root: &Path, max_items: usize) -> Result<DriveReport> {
    crate::app_state::require_existing(root)?;

    let mut report = DriveReport {
        root: root.to_path_buf(),
        ..Default::default()
    };

    let mut by_category: HashMap<FileCategory, (usize, u64)> = HashMap::new();
    let mut by_signature: HashMap<(String, u64), Vec<PathBuf>> = HashMap::new();
    let mut all_files: Vec<LargeFile> = Vec::new();

    for entry in WalkDir::new(root).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                report
                    .warnings
                    .push(Notice::read_failed(root.display(), err));
                continue;
            }
        };

        if entry.file_type().is_dir() {
            report.dir_count += 1;
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let size = match entry.metadata() {
            Ok(meta) => meta.len(),
            Err(err) => {
                report
                    .warnings
                    .push(Notice::read_failed(path.display(), err));
                continue;
            }
        };
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        report.file_count += 1;
        report.total_bytes += size;

        let category = FileCategory::from_path(path);
        let stats = by_category.entry(category).or_insert((0, 0));
        stats.0 += 1;
        stats.1 += size;

        if category == FileCategory::Placeholder {
            report.placeholder_count += 1;
            if report.placeholders.len() < max_items {
                report.placeholders.push(PlaceholderFile {
                    kind: path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase(),
                    target_url: read_placeholder_url(path),
                    file_name: file_name.clone(),
                    path: path.to_path_buf(),
                });
            }
        }

        // Zero-length duplicates are noise: exported empty folders, placeholder
        // files, synchronisation artefacts.
        if size > 0 {
            by_signature
                .entry((file_name.clone(), size))
                .or_default()
                .push(path.to_path_buf());
        }

        all_files.push(LargeFile {
            path: path.to_path_buf(),
            file_name,
            size_bytes: size,
        });
    }

    report.categories = by_category
        .into_iter()
        .map(|(category, (file_count, total_bytes))| CategoryStats {
            category,
            file_count,
            total_bytes,
        })
        .collect();
    report
        .categories
        .sort_by_key(|stats| std::cmp::Reverse(stats.total_bytes));

    let mut duplicates: Vec<DuplicateGroup> = by_signature
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|((file_name, size_bytes), paths)| {
            // Every copy past the first is reclaimable space.
            report.duplicate_bytes += size_bytes * (paths.len() as u64 - 1);
            DuplicateGroup {
                file_name,
                size_bytes,
                paths,
            }
        })
        .collect();
    duplicates.sort_by(|a, b| {
        let a_waste = a.size_bytes * (a.paths.len() as u64 - 1);
        let b_waste = b.size_bytes * (b.paths.len() as u64 - 1);
        b_waste.cmp(&a_waste)
    });
    duplicates.truncate(max_items);
    report.duplicate_groups = duplicates;

    all_files.sort_by_key(|file| std::cmp::Reverse(file.size_bytes));
    all_files.truncate(max_items);
    report.largest_files = all_files;

    if report.placeholder_count > 0 {
        report.warnings.push(Notice::PlaceholdersWithoutContent {
            count: report.placeholder_count,
        });
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Cleanup
// ---------------------------------------------------------------------------

/// How to treat the files being removed.
///
/// A deleting mode is missing on purpose: an export is often the only copy left
/// of that data, and a botched deduplication carried out as a deletion cannot be
/// undone. Both working modes produce something that can be taken back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanMode {
    /// Computes the plan without touching anything.
    DryRun,
    /// Builds a clean tree elsewhere, leaving the source untouched.
    CopyToOutput,
    /// Moves junk and surplus copies into a quarantine folder, writing a ledger
    /// that allows putting everything back.
    Quarantine,
}

/// Why a file was moved to quarantine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuarantineReason {
    Junk,
    Duplicate,
    /// A companion file following the media that was removed.
    Companion,
    /// A sidecar whose content is now inside the media, which stays where it is.
    AppliedSidecar,
}

/// Cleanup parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanOptions {
    pub mode: CleanMode,
    /// Output tree, or quarantine root, depending on the mode.
    pub destination: Option<PathBuf>,
    pub remove_junk: bool,
    pub remove_duplicates: bool,
    /// Take companion files along when a media file is removed.
    ///
    /// Needed on Google Photos: removing `IMG_1268 2.JPG` and leaving
    /// `IMG_1268 2.JPG.supplemental-metadata.json` behind produces an orphan
    /// sidecar that no longer describes anything.
    pub move_companions: bool,
}

impl Default for CleanOptions {
    fn default() -> Self {
        Self {
            mode: CleanMode::DryRun,
            destination: None,
            remove_junk: true,
            remove_duplicates: true,
            move_companions: true,
        }
    }
}

/// A group of files with identical content, verified by hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentDuplicateGroup {
    /// BLAKE3 hash of the content, shortened.
    pub hash: String,
    pub size_bytes: u64,
    /// The copy that gets kept.
    pub kept: PathBuf,
    /// The surplus copies.
    pub copies: Vec<PathBuf>,
}

/// The cleanup plan: what would happen, without it having happened.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanPlan {
    pub root: PathBuf,
    pub files_scanned: usize,
    /// Files that would stay where they are.
    pub files_kept: usize,
    pub duplicate_copies: usize,
    pub junk_files: usize,
    /// Sidecars and the like that will follow the media removed.
    pub companion_files: usize,
    pub reclaimable_bytes: u64,
    /// Bytes actually read in order to compute the hashes.
    pub hashed_bytes: u64,
    pub duplicate_groups: Vec<ContentDuplicateGroup>,
    pub junk_sample: Vec<PathBuf>,
    pub warnings: Vec<Notice>,
}

/// One entry of the quarantine ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineEntry {
    pub original: PathBuf,
    pub quarantined: PathBuf,
    pub reason: QuarantineReason,
    pub size_bytes: u64,
}

/// The ledger written into the quarantine, which makes the operation reversible.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineManifest {
    pub created_at: DateTime<Utc>,
    pub source_root: PathBuf,
    pub entries: Vec<QuarantineEntry>,
}

/// Outcome of a cleanup that was carried out.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanReport {
    pub mode: Option<CleanMode>,
    pub destination: Option<PathBuf>,
    pub files_kept: usize,
    pub duplicates_handled: usize,
    pub junk_handled: usize,
    /// Sidecars moved along with the media they belonged to.
    pub companions_handled: usize,
    pub bytes_reclaimed: u64,
    /// Path of the ledger, present only in quarantine mode.
    pub manifest: Option<PathBuf>,
    pub failures: Vec<String>,
}

/// Outcome of a restore from quarantine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub restored: usize,
    pub skipped_existing: usize,
    pub failures: Vec<String>,
}

/// True if the file is an operating system artefact.
pub fn is_junk(path: &Path) -> bool {
    // A service folder contaminates everything it contains.
    if path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(|part| JUNK_DIRS.contains(&part))
    {
        return true;
    }

    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };

    JUNK_NAMES
        .iter()
        .any(|junk| name.eq_ignore_ascii_case(junk))
        || JUNK_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// Names of the files inside each folder, collected during the scan.
///
/// It exists so companion files can be found without rereading the folder.
type DirIndex = HashMap<PathBuf, Vec<String>>;

/// True if the file sits beside another one in the same folder, that is if its
/// name is that file's complete name plus a suffix.
///
/// It is the reverse of [`find_companions`]: it exists to exclude sidecars from
/// deduplication, because they are not files in their own right.
fn is_companion(path: &Path, index: &DirIndex) -> bool {
    let (Some(name), Some(parent)) = (path.file_name().and_then(|n| n.to_str()), path.parent())
    else {
        return false;
    };
    let Some(names) = index.get(parent) else {
        return false;
    };

    names.iter().any(|candidate| {
        candidate.len() < name.len()
            && name.starts_with(candidate.as_str())
            && name.as_bytes().get(candidate.len()) == Some(&b'.')
    })
}

/// Finds the files sitting beside a media file, that is those whose name is the
/// media's complete name followed by a suffix.
///
/// That is the Google Photos sidecar convention: `IMG_1268 2.JPG` is accompanied
/// by `IMG_1268 2.JPG.supplemental-metadata.json`. The rule is deliberately
/// strict, matching the complete name including extension, so `IMG_1268.JPG`
/// does not accidentally capture the files of `IMG_1268 2.JPG`.
///
/// The search runs on the in-memory index rather than the filesystem: the
/// version that reread the folder on every call cost the product of the number
/// of duplicates and the number of files in their folder, and on a library of
/// twenty thousand photos it took the cleanup plan from one second to nearly
/// seven minutes.
fn find_companions(media: &Path, index: &DirIndex) -> Vec<PathBuf> {
    let Some(name) = media.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    let Some(parent) = media.parent() else {
        return Vec::new();
    };
    let Some(names) = index.get(parent) else {
        return Vec::new();
    };
    let prefix = format!("{name}.");

    names
        .iter()
        .filter(|candidate| candidate.starts_with(&prefix))
        .map(|candidate| parent.join(candidate))
        .collect()
}

/// Computes the BLAKE3 hash of the content, reading in blocks.
fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = std::fs::File::open(path).map_err(|e| TakeoutError::io(path, e))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; HASH_BUFFER];
    let mut read_total = 0u64;

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| TakeoutError::io(path, e))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        read_total += read as u64;
    }

    Ok((hasher.finalize().to_hex()[..16].to_string(), read_total))
}

/// Chooses which copy to keep in a group of duplicates.
///
/// The shortest path wins, ties broken alphabetically. That is not arbitrary:
/// the copies generated by operating systems and by Google add suffixes
/// (`IMG_1268 2.JPG`, `document (1).pdf`), so the shortest name is almost always
/// the original.
fn choose_kept(paths: &mut Vec<PathBuf>) -> PathBuf {
    paths.sort_by(|a, b| {
        let a_str = a.to_string_lossy();
        let b_str = b.to_string_lossy();
        a_str
            .len()
            .cmp(&b_str.len())
            .then_with(|| a_str.cmp(&b_str))
    });
    paths.remove(0)
}

/// Builds the cleanup plan without modifying anything.
///
/// Deduplication happens in two phases: first files are grouped by size, which
/// is free, then the hash is computed only for groups holding more than one
/// file. On an export where nearly every file is unique, that avoids reading
/// the entire content of the disk.
pub fn plan_clean(
    root: &Path,
    options: &CleanOptions,
    max_items: usize,
    progress: ProgressSink<'_>,
) -> Result<CleanPlan> {
    crate::app_state::require_existing(root)?;

    let mut plan = CleanPlan {
        root: root.to_path_buf(),
        ..Default::default()
    };

    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut junk: Vec<PathBuf> = Vec::new();
    let mut dir_index: DirIndex = HashMap::new();

    // The index has to be complete before deciding what can be deduplicated: to
    // recognise a sidecar you need to know whether the media it belongs to exists,
    // and that media may come after it in scan order.
    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        if let (Some(parent), Some(name)) = (
            entry.path().parent(),
            entry.path().file_name().and_then(|n| n.to_str()),
        ) {
            dir_index
                .entry(parent.to_path_buf())
                .or_default()
                .push(name.to_string());
        }
    }

    for entry in WalkDir::new(root).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                plan.warnings.push(Notice::read_failed(root.display(), err));
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        plan.files_scanned += 1;
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

        // The index is filled here, taking advantage of a pass we are making
        // anyway, rather than rereading the folders later on.
        if let (Some(parent), Some(name)) =
            (path.parent(), path.file_name().and_then(|n| n.to_str()))
        {
            dir_index
                .entry(parent.to_path_buf())
                .or_default()
                .push(name.to_string());
        }

        if options.remove_junk && is_junk(path) {
            plan.reclaimable_bytes += size;
            junk.push(path.to_path_buf());
            continue;
        }

        // A companion file has no life of its own: it belongs to its media and
        // follows it when that is moved. Treating it as an independent candidate
        // would allow removing one because another photo's sidecar happens to have
        // the same content, leaving that photo without its metadata. It is a
        // mistake you cannot recover from by looking at the files left behind.
        if options.move_companions && is_companion(path, &dir_index) {
            plan.files_kept += 1;
            continue;
        }

        // Empty files all share the same hash: grouping them would produce one
        // enormous group of "duplicates" that are not duplicates in any useful sense.
        if options.remove_duplicates && size > 0 {
            by_size.entry(size).or_default().push(path.to_path_buf());
        } else {
            plan.files_kept += 1;
        }
    }

    plan.junk_files = junk.len();
    plan.junk_sample = junk.into_iter().take(50).collect();

    // Only groups holding more than one file per size deserve a hash.
    let candidates: Vec<(u64, Vec<PathBuf>)> = by_size
        .into_iter()
        .filter(|(_, paths)| {
            if paths.len() > 1 {
                true
            } else {
                plan.files_kept += paths.len();
                false
            }
        })
        .collect();

    let to_hash: usize = candidates.iter().map(|(_, p)| p.len()).sum();
    trace_dev!(
        "cleanup: {} files examined, {} to verify by content",
        plan.files_scanned,
        to_hash
    );
    progress(Progress::new(Phase::Scanning, 0, to_hash, 0));

    let done = std::sync::atomic::AtomicUsize::new(0);
    use std::sync::atomic::Ordering::Relaxed;

    let hashed: Vec<(u64, String, PathBuf, u64)> = candidates
        .par_iter()
        .flat_map(|(size, paths)| {
            paths
                .par_iter()
                .filter_map(|path| {
                    let outcome = hash_file(path)
                        .ok()
                        .map(|(hash, read)| (*size, hash, path.clone(), read));
                    let seen = done.fetch_add(1, Relaxed) + 1;
                    progress(
                        Progress::new(Phase::Scanning, seen, to_hash, 0)
                            .with_current(path.file_name().unwrap_or_default().to_string_lossy()),
                    );
                    outcome
                })
                .collect::<Vec<_>>()
        })
        .collect();

    let mut by_hash: HashMap<(u64, String), Vec<PathBuf>> = HashMap::new();
    for (size, hash, path, read) in hashed {
        plan.hashed_bytes += read;
        by_hash.entry((size, hash)).or_default().push(path);
    }

    for ((size_bytes, hash), mut paths) in by_hash {
        if paths.len() == 1 {
            plan.files_kept += 1;
            continue;
        }

        let kept = choose_kept(&mut paths);
        plan.files_kept += 1;
        plan.duplicate_copies += paths.len();
        plan.reclaimable_bytes += size_bytes * paths.len() as u64;
        plan.duplicate_groups.push(ContentDuplicateGroup {
            hash,
            size_bytes,
            kept,
            copies: paths,
        });
    }

    if options.move_companions {
        for group in &plan.duplicate_groups {
            for copy in &group.copies {
                for companion in find_companions(copy, &dir_index) {
                    plan.companion_files += 1;
                    plan.reclaimable_bytes +=
                        std::fs::metadata(&companion).map(|m| m.len()).unwrap_or(0);
                }
            }
        }
    }

    plan.duplicate_groups
        .sort_by_key(|g| std::cmp::Reverse(g.size_bytes * g.copies.len() as u64));

    // The counts stay complete, the list does not: the list is what crosses
    // the IPC channel towards the interface, and on a real library it would
    // become a few megabytes of JSON on every scan.
    plan.duplicate_groups.truncate(max_items);

    progress(Progress::new(Phase::Done, to_hash, to_hash, 0));
    Ok(plan)
}

/// Checks that the destination does not sit inside the source.
fn check_destination(root: &Path, destination: &Path) -> Result<()> {
    if destination.starts_with(root) {
        return Err(TakeoutError::DestinationInsideSource);
    }
    Ok(())
}

/// Performs the cleanup according to the options given.
pub fn clean(
    root: &Path,
    options: &CleanOptions,
    progress: ProgressSink<'_>,
) -> Result<CleanReport> {
    // Here the list of duplicates is needed whole, not truncated for the UI.
    let plan = plan_clean(root, options, usize::MAX, progress)?;

    let mut report = CleanReport {
        mode: Some(options.mode),
        destination: options.destination.clone(),
        files_kept: plan.files_kept,
        ..Default::default()
    };

    if options.mode == CleanMode::DryRun {
        return Ok(report);
    }

    let destination = options
        .destination
        .as_ref()
        .ok_or(TakeoutError::DestinationRequired)?;
    check_destination(root, destination)?;

    // In quarantine mode files are moved, so space is needed only when the
    // destination is on another volume; building a clean tree instead duplicates
    // nearly everything, and it is better to know that before starting.
    if options.mode == CleanMode::CopyToOutput {
        // The clean tree holds everything except what gets discarded, so the space
        // needed is the total minus the reclaimable part.
        let total: u64 = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        let da_scrivere = total.saturating_sub(plan.reclaimable_bytes);
        crate::app_state::require_free_space(destination, da_scrivere)?;
    }

    std::fs::create_dir_all(destination).map_err(|e| TakeoutError::io(destination, e))?;

    // The set of files to remove: the surplus copies and the junk.
    let mut dir_index: DirIndex = HashMap::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        if let (Some(parent), Some(name)) = (
            entry.path().parent(),
            entry.path().file_name().and_then(|n| n.to_str()),
        ) {
            dir_index
                .entry(parent.to_path_buf())
                .or_default()
                .push(name.to_string());
        }
    }

    let mut removable: Vec<(PathBuf, QuarantineReason)> = Vec::new();
    for group in &plan.duplicate_groups {
        for copy in &group.copies {
            removable.push((copy.clone(), QuarantineReason::Duplicate));
            if options.move_companions {
                for companion in find_companions(copy, &dir_index) {
                    removable.push((companion, QuarantineReason::Companion));
                }
            }
        }
    }
    for junk in &plan.junk_sample {
        removable.push((junk.clone(), QuarantineReason::Junk));
    }
    // `junk_sample` is truncated for the UI: here the complete list is needed.
    if plan.junk_files > plan.junk_sample.len() {
        for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
            if entry.file_type().is_file()
                && is_junk(entry.path())
                && !plan.junk_sample.iter().any(|p| p == entry.path())
            {
                removable.push((entry.into_path(), QuarantineReason::Junk));
            }
        }
    }

    match options.mode {
        CleanMode::CopyToOutput => {
            let excluded: std::collections::HashSet<&PathBuf> =
                removable.iter().map(|(path, _)| path).collect();

            for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                if excluded.contains(&path.to_path_buf()) {
                    continue;
                }

                let relative = path.strip_prefix(root).unwrap_or(path);
                let target = destination.join(relative);
                if let Some(parent) = target.parent() {
                    if let Err(err) = std::fs::create_dir_all(parent) {
                        report.failures.push(format!("{}: {err}", parent.display()));
                        continue;
                    }
                }
                match std::fs::copy(path, &target) {
                    Ok(_) => {}
                    Err(err) => report.failures.push(format!("{}: {err}", path.display())),
                }
            }

            report.duplicates_handled = plan.duplicate_copies;
            report.junk_handled = plan.junk_files;
            report.companions_handled = plan.companion_files;
            report.bytes_reclaimed = plan.reclaimable_bytes;
        }

        CleanMode::Quarantine => {
            let mut manifest = QuarantineManifest {
                created_at: Utc::now(),
                source_root: root.to_path_buf(),
                entries: Vec::new(),
            };

            for (path, reason) in &removable {
                let relative = path.strip_prefix(root).unwrap_or(path);
                let target = destination.join(relative);

                if let Some(parent) = target.parent() {
                    if let Err(err) = std::fs::create_dir_all(parent) {
                        report.failures.push(format!("{}: {err}", parent.display()));
                        continue;
                    }
                }

                let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                // `rename` fails across volumes: in that case we copy and remove, which
                // is the only way to move between filesystems.
                let moved = std::fs::rename(path, &target).or_else(|_| {
                    std::fs::copy(path, &target).and_then(|_| std::fs::remove_file(path))
                });

                match moved {
                    Ok(_) => {
                        report.bytes_reclaimed += size;
                        match reason {
                            QuarantineReason::Duplicate => report.duplicates_handled += 1,
                            QuarantineReason::Junk => report.junk_handled += 1,
                            QuarantineReason::Companion => report.companions_handled += 1,
                            // Cleanup never produces this reason: it comes only from
                            // `sweep_applied_sidecars`, which writes its own
                            // ledger.
                            QuarantineReason::AppliedSidecar => {}
                        }
                        manifest.entries.push(QuarantineEntry {
                            original: path.clone(),
                            quarantined: target,
                            reason: *reason,
                            size_bytes: size,
                        });
                    }
                    Err(err) => report.failures.push(format!("{}: {err}", path.display())),
                }
            }

            // The ledger has to be written even if some moves failed: without it,
            // what has already been moved cannot be recovered.
            let manifest_path = destination.join(MANIFEST_NAME);
            let json = serde_json::to_string_pretty(&manifest)
                .map_err(|e| TakeoutError::Metadata(e.to_string()))?;
            std::fs::write(&manifest_path, json)
                .map_err(|e| TakeoutError::io(&manifest_path, e))?;
            report.manifest = Some(manifest_path);
        }

        CleanMode::DryRun => unreachable!("gestito prima"),
    }

    trace_dev!(
        "cleanup finished: {} duplicates, {} junk, {} bytes, {} errors",
        report.duplicates_handled,
        report.junk_handled,
        report.bytes_reclaimed,
        report.failures.len()
    );

    Ok(report)
}

/// How many times a retention reason occurs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptReason {
    pub reason: crate::exif_parser::SidecarKept,
    pub count: usize,
}

/// Outcome of setting aside the sidecars whose content is now in the files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarSweepReport {
    pub destination: PathBuf,
    pub moved: usize,
    pub bytes_moved: u64,
    /// Sidecars left where they were because still the sole copy of something.
    pub kept: usize,
    /// Why they were left, with how many times each reason occurs.
    pub kept_reasons: Vec<KeptReason>,
    /// Sample of the files left behind, so they can be looked at.
    pub kept_sample: Vec<PathBuf>,
    pub manifest: Option<PathBuf>,
    pub failures: Vec<String>,
}

/// Moves the sidecars whose content is now inside the media they belong to.
///
/// This is not a cleanup: it is the last step of a successful repair. A JSON
/// beside a photo that now carries the same data adds nothing, but until it has
/// been verified that the data really is there, the JSON is the only copy of
/// something and must be left alone.
///
/// That is why the decision is not based on what the repair reported but on what
/// is written in the file right now, read one by one. Left behind are:
///
/// - the sidecars of PNG, GIF and video files, formats where we write no EXIF
///   and for which the JSON is therefore the only home of date and coordinates;
/// - those whose media does not appear to be repaired yet;
/// - those carrying data with no home in the metadata, such as the Google Photos
///   view count.
///
/// The move is reversible: it writes the same ledger as quarantine, and
/// [`restore_quarantine`] puts every file back in its place.
pub fn sweep_applied_sidecars(
    root: &Path,
    destination: &Path,
    max_items: usize,
    progress: crate::app_state::ProgressSink<'_>,
) -> Result<SidecarSweepReport> {
    use crate::app_state::{Phase, Progress};
    use crate::exif_parser;

    crate::app_state::require_existing(root)?;
    let root = root
        .canonicalize()
        .map_err(|e| crate::app_state::TakeoutError::io(root, e))?;

    // Index of the existing paths: without it every sidecar candidate would cost
    // a filesystem access inside folders holding tens of thousands of entries.
    let mut index = exif_parser::FileIndex::new();
    let mut media: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.into_path();
        if exif_parser::is_media_file(&path) {
            media.push(path.clone());
        }
        index.insert(path);
    }

    let total = media.len();
    progress(Progress::new(Phase::Scanning, 0, total, 0));

    let mut report = SidecarSweepReport {
        destination: destination.to_path_buf(),
        ..Default::default()
    };
    let mut manifest = QuarantineManifest {
        created_at: Utc::now(),
        source_root: root.clone(),
        entries: Vec::new(),
    };
    // Sorted, so the list stays stable between one run and the next.
    let mut counts: BTreeMap<exif_parser::SidecarKept, usize> = BTreeMap::new();

    for (fatti, file) in media.iter().enumerate() {
        progress(Progress::new(Phase::Writing, fatti, total, 0));

        let Ok(Some(sidecar)) = exif_parser::read_sidecar(file, Some(&index)) else {
            continue;
        };

        let mut reasons = exif_parser::sidecar_residual(file, &sidecar)?;
        // What has no home in the metadata remains a valid reason not to touch the
        // JSON, even though no repair will ever be able to resolve it.
        reasons.extend(sidecar.unwritable());

        if !reasons.is_empty() {
            report.kept += 1;
            for motivo in reasons {
                *counts.entry(motivo).or_insert(0) += 1;
            }
            if report.kept_sample.len() < max_items {
                report.kept_sample.push(sidecar.path.clone());
            }
            continue;
        }

        let relative = sidecar.path.strip_prefix(&root).unwrap_or(&sidecar.path);
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                report.failures.push(format!("{}: {err}", parent.display()));
                continue;
            }
        }

        let size = std::fs::metadata(&sidecar.path)
            .map(|m| m.len())
            .unwrap_or(0);
        // `rename` fails across volumes: then we copy and remove.
        let moved = std::fs::rename(&sidecar.path, &target).or_else(|_| {
            std::fs::copy(&sidecar.path, &target).and_then(|_| std::fs::remove_file(&sidecar.path))
        });

        match moved {
            Ok(_) => {
                report.moved += 1;
                report.bytes_moved += size;
                manifest.entries.push(QuarantineEntry {
                    original: sidecar.path.clone(),
                    quarantined: target,
                    reason: QuarantineReason::AppliedSidecar,
                    size_bytes: size,
                });
            }
            Err(err) => report
                .failures
                .push(format!("{}: {err}", sidecar.path.display())),
        }
    }

    report.kept_reasons = counts
        .into_iter()
        .map(|(reason, count)| KeptReason { reason, count })
        .collect();

    // The ledger has to be written even if some moves failed: without it, what
    // has already been moved cannot be recovered.
    if !manifest.entries.is_empty() {
        let manifest_path = destination.join(MANIFEST_NAME);
        let json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| crate::app_state::TakeoutError::Metadata(e.to_string()))?;
        std::fs::write(&manifest_path, json)
            .map_err(|e| crate::app_state::TakeoutError::io(&manifest_path, e))?;
        report.manifest = Some(manifest_path);
    }

    progress(Progress::new(Phase::Done, total, total, 0));
    trace_dev!(
        "sidecars: {} moved, {} kept, {} errors",
        report.moved,
        report.kept,
        report.failures.len()
    );

    Ok(report)
}

/// Puts the files moved to quarantine back where they were.
///
/// It is the function that makes the word "reversible" true: without it,
/// quarantine would be a deletion with a friendlier name.
pub fn restore_quarantine(manifest_path: &Path) -> Result<RestoreReport> {
    let content =
        std::fs::read_to_string(manifest_path).map_err(|e| TakeoutError::io(manifest_path, e))?;
    let manifest: QuarantineManifest = serde_json::from_str(&content)
        .map_err(|e| TakeoutError::Metadata(format!("{}: {e}", manifest_path.display())))?;

    let mut report = RestoreReport::default();

    for entry in &manifest.entries {
        // If something has reappeared at the origin meanwhile, we do not overwrite
        // it: better to leave the file in quarantine and say so.
        if entry.original.exists() {
            report.skipped_existing += 1;
            continue;
        }
        if let Some(parent) = entry.original.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                report.failures.push(format!("{}: {err}", parent.display()));
                continue;
            }
        }

        let moved = std::fs::rename(&entry.quarantined, &entry.original).or_else(|_| {
            std::fs::copy(&entry.quarantined, &entry.original)
                .and_then(|_| std::fs::remove_file(&entry.quarantined))
        });

        match moved {
            Ok(_) => report.restored += 1,
            Err(err) => report
                .failures
                .push(format!("{}: {err}", entry.quarantined.display())),
        }
    }

    trace_dev!(
        "restore: {} put back, {} skipped, {} errors",
        report.restored,
        report.skipped_existing,
        report.failures.len()
    );

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Setting sidecars aside has to be selective and undoable.
    ///
    /// Selective because a JSON beside a file that does not yet carry that data
    /// is the only copy of something: moving it would be a loss dressed up as
    /// a cleanup. Undoable because "nothing is ever deleted" is worth little
    /// if the file then cannot be put back where it was.
    #[test]
    fn moves_only_sidecars_whose_content_is_already_in_the_file() {
        use crate::app_state::testing::{write_bytes, write_file, TempDir, MINIMAL_JPEG};
        use crate::exif_parser::{apply_metadata, WriteMode, WriteOptions};

        let temp = TempDir::new("sidecar-spostati");
        let photos = temp.path().join("Google Foto");

        let sidecar = |name: &str| {
            format!(
                r#"{{"title": "{name}",
                     "photoTakenTime": {{ "timestamp": "1577880000" }},
                     "geoData": {{ "latitude": 45.4642, "longitude": 9.19, "altitude": 0.0 }} }}"#
            )
        };

        // Repairable: once rewritten, the JSON is no longer needed.
        write_bytes(&photos.join("IMG_0001.JPG"), MINIMAL_JPEG);
        write_file(&photos.join("IMG_0001.JPG.json"), &sidecar("IMG_0001.JPG"));

        // Video: we write no EXIF, so the JSON stays the only home.
        write_file(&photos.join("VID_0002.mp4"), "not a real video");
        write_file(&photos.join("VID_0002.mp4.json"), &sidecar("VID_0002.mp4"));

        // Never repaired: the date lives only in the JSON.
        write_bytes(&photos.join("IMG_0003.JPG"), MINIMAL_JPEG);
        write_file(&photos.join("IMG_0003.JPG.json"), &sidecar("IMG_0003.JPG"));

        // With a Google counter, which has no home in the metadata.
        write_bytes(&photos.join("IMG_0004.JPG"), MINIMAL_JPEG);
        write_file(
            &photos.join("IMG_0004.JPG.json"),
            r#"{"title": "IMG_0004.JPG",
                "photoTakenTime": { "timestamp": "1577880000" },
                "imageViews": "128"}"#,
        );

        // Repair only the first two photos, leaving IMG_0003 behind.
        let da_riparare = temp.path().join("solo-alcune");
        for name in ["IMG_0001.JPG", "IMG_0004.JPG"] {
            write_bytes(&da_riparare.join(name), MINIMAL_JPEG);
            std::fs::copy(
                photos.join(format!("{name}.json")),
                da_riparare.join(format!("{name}.json")),
            )
            .expect("copy sidecar");
        }
        apply_metadata(
            &photos,
            &WriteOptions {
                mode: WriteMode::InPlace,
                ..WriteOptions::default()
            },
            &crate::app_state::no_progress,
        )
        .expect("repair");
        // Put IMG_0003 back to its starting state: repaired is not what we want.
        write_bytes(&photos.join("IMG_0003.JPG"), MINIMAL_JPEG);

        let quarantena = temp.path().join("sidecar-applicati");
        let report =
            sweep_applied_sidecars(&photos, &quarantena, 10, &crate::app_state::no_progress)
                .expect("sweep");

        assert_eq!(report.moved, 1, "only IMG_0001 is fully repaired");
        assert_eq!(report.kept, 3, "the other three stay: {report:?}");
        assert!(
            !photos.join("IMG_0001.JPG.json").exists(),
            "the applied sidecar has to be moved"
        );
        assert!(
            photos.join("VID_0002.mp4.json").exists(),
            "with no EXIF the JSON is the only home: leave it alone"
        );
        assert!(
            photos.join("IMG_0003.JPG.json").exists(),
            "an unrepaired file does not lose its sidecar"
        );
        assert!(
            photos.join("IMG_0004.JPG.json").exists(),
            "data with no home in the metadata holds the sidecar back"
        );
        assert!(
            report
                .kept_reasons
                .iter()
                .any(|m| m.reason == crate::exif_parser::SidecarKept::ViewCountHasNoTag),
            "the reason has to be stated: {:?}",
            report.kept_reasons
        );

        // And it must be possible to go back.
        let manifest = report.manifest.expect("registro scritto");
        let restored = restore_quarantine(&manifest).expect("restored");
        assert_eq!(restored.restored, 1);
        assert!(
            photos.join("IMG_0001.JPG.json").exists(),
            "the restore puts the sidecar back where it was"
        );
    }

    #[test]
    fn classifies_by_extension() {
        assert_eq!(
            FileCategory::from_path(Path::new("relazione.docx")),
            FileCategory::Document
        );
        assert_eq!(
            FileCategory::from_path(Path::new("bilancio.xlsx")),
            FileCategory::Spreadsheet
        );
        assert_eq!(
            FileCategory::from_path(Path::new("photos.HEIC")),
            FileCategory::Image
        );
        assert_eq!(
            FileCategory::from_path(Path::new("senza_estensione")),
            FileCategory::Other
        );
    }

    use crate::app_state::no_progress;
    use crate::app_state::testing::{write_file, TempDir};

    /// Test tree with real duplicates, size lookalikes and junk.
    fn build_drive(root: &Path) -> PathBuf {
        let drive = root.join("Drive");

        // Same content, different names and folders: genuine duplicates.
        write_file(&drive.join("relazione.docx"), "contenuto A");
        write_file(&drive.join("copy").join("relazione.docx"), "contenuto A");

        // Same size as the previous ones but different content: deduplication by
        // name and size would get these wrong, deduplication by content does not.
        write_file(&drive.join("altro.txt"), "contenuto B");
        write_file(&drive.join("terzo.txt"), "contenuto C");

        // System junk.
        write_file(&drive.join(".DS_Store"), "spazzatura");
        write_file(&drive.join("sub").join("._nascosto"), "appledouble");
        write_file(&drive.join("__MACOSX").join("roba.txt"), "spazzatura");

        drive
    }

    #[test]
    fn recognises_system_junk() {
        assert!(is_junk(Path::new("/x/.DS_Store")));
        assert!(is_junk(Path::new("/x/desktop.ini")));
        assert!(is_junk(Path::new("/x/Thumbs.db")));
        assert!(is_junk(Path::new("/x/._foto.jpg")));
        assert!(is_junk(Path::new("/x/__MACOSX/qualsiasi.txt")));
        // The real files must not be touched.
        assert!(!is_junk(Path::new("/x/relazione.docx")));
        assert!(!is_junk(Path::new("/x/.gitignore")));
    }

    #[test]
    fn keeps_the_copy_with_the_shortest_name() {
        let mut paths = vec![
            PathBuf::from("/photos/IMG_1268 2.JPG"),
            PathBuf::from("/photos/IMG_1268.JPG"),
        ];
        // The " 2" suffix marks the copy, not the original.
        assert_eq!(
            choose_kept(&mut paths),
            PathBuf::from("/photos/IMG_1268.JPG")
        );
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn tells_real_duplicates_from_size_lookalikes() {
        let temp = TempDir::new("drive-plan");
        let drive = build_drive(temp.path());

        let plan =
            plan_clean(&drive, &CleanOptions::default(), usize::MAX, &no_progress).expect("plan");

        assert_eq!(plan.files_scanned, 7);
        assert_eq!(plan.junk_files, 3, "DS_Store, AppleDouble e __MACOSX");
        assert_eq!(
            plan.duplicate_groups.len(),
            1,
            "only relazione.docx is a genuine duplicate"
        );
        assert_eq!(plan.duplicate_copies, 1);

        // The two files of equal size but different content both stay.
        let gruppo = &plan.duplicate_groups[0];
        assert!(gruppo.kept.ends_with("relazione.docx"));
        assert_eq!(gruppo.copies.len(), 1);
        assert!(gruppo.copies[0].ends_with("copy/relazione.docx"));

        // The plan touched nothing.
        assert!(drive.join("copy").join("relazione.docx").is_file());
        assert!(drive.join(".DS_Store").is_file());
    }

    #[test]
    fn the_dry_run_writes_nothing() {
        let temp = TempDir::new("drive-simulazione");
        let drive = build_drive(temp.path());
        let prima: Vec<PathBuf> = WalkDir::new(&drive)
            .into_iter()
            .flatten()
            .map(|e| e.into_path())
            .collect();

        clean(&drive, &CleanOptions::default(), &no_progress).expect("simulazione");

        let dopo: Vec<PathBuf> = WalkDir::new(&drive)
            .into_iter()
            .flatten()
            .map(|e| e.into_path())
            .collect();
        assert_eq!(prima, dopo, "the dry run has to leave the tree untouched");
    }

    #[test]
    fn quarantine_undoes_completely() {
        let temp = TempDir::new("drive-quarantena");
        let drive = build_drive(temp.path());
        let quarantena = temp.path().join("quarantena");

        /// Snapshot of paths and contents, to compare before and after.
        fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
            let mut out: Vec<(PathBuf, Vec<u8>)> = WalkDir::new(root)
                .into_iter()
                .flatten()
                .filter(|e| e.file_type().is_file())
                .map(|e| {
                    let content = std::fs::read(e.path()).unwrap_or_default();
                    (e.into_path(), content)
                })
                .collect();
            out.sort();
            out
        }

        let prima = snapshot(&drive);
        assert_eq!(prima.len(), 7);

        let report = clean(
            &drive,
            &CleanOptions {
                mode: CleanMode::Quarantine,
                destination: Some(quarantena.clone()),
                ..Default::default()
            },
            &no_progress,
        )
        .expect("quarantena");

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.duplicates_handled, 1);
        assert_eq!(report.junk_handled, 3);

        // The files were moved, not deleted.
        assert!(!drive.join("copy").join("relazione.docx").exists());
        assert!(!drive.join(".DS_Store").exists());
        assert_eq!(snapshot(&drive).len(), 3, "the three unique files remain");
        assert!(quarantena.join("copy").join("relazione.docx").is_file());

        // The ledger exists and can be read.
        let manifest = report.manifest.expect("registro scritto");
        assert!(manifest.is_file());

        // And now the part that counts: back to exactly where we started.
        let restore = restore_quarantine(&manifest).expect("restored");
        assert_eq!(restore.restored, 4);
        assert!(restore.failures.is_empty(), "{:?}", restore.failures);
        assert_eq!(
            snapshot(&drive),
            prima,
            "after the restore the tree has to be identical to the original"
        );
    }

    #[test]
    fn the_clean_tree_excludes_duplicates_and_junk() {
        let temp = TempDir::new("drive-copy");
        let drive = build_drive(temp.path());
        let output = temp.path().join("pulito");

        let report = clean(
            &drive,
            &CleanOptions {
                mode: CleanMode::CopyToOutput,
                destination: Some(output.clone()),
                ..Default::default()
            },
            &no_progress,
        )
        .expect("albero pulito");

        assert!(report.failures.is_empty(), "{:?}", report.failures);

        let produced: Vec<String> = WalkDir::new(&output)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                e.path()
                    .strip_prefix(&output)
                    .unwrap_or(e.path())
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();

        assert_eq!(produced.len(), 3, "one specimen per content: {produced:?}");
        assert!(produced.iter().any(|p| p == "relazione.docx"));
        assert!(produced.iter().any(|p| p == "altro.txt"));
        assert!(produced.iter().any(|p| p == "terzo.txt"));
        assert!(!produced.iter().any(|p| p.contains("DS_Store")));
        assert!(!produced.iter().any(|p| p.contains("__MACOSX")));

        // The source was not touched.
        assert!(drive.join("copy").join("relazione.docx").is_file());
        assert!(drive.join(".DS_Store").is_file());
    }

    #[test]
    fn the_sidecar_follows_the_removed_media() {
        let temp = TempDir::new("drive-sidecar");
        let photos = temp.path().join("Google Foto");

        // Two shots with identical content, the way Google produces them when the
        // same photo sits in several albums, each with its own sidecar.
        write_file(&photos.join("IMG_1268.JPG"), "pixel identici");
        write_file(
            &photos.join("IMG_1268.JPG.supplemental-metadata.json"),
            r#"{"title": "IMG_1268.JPG"}"#,
        );
        write_file(&photos.join("IMG_1268 2.JPG"), "pixel identici");
        write_file(
            &photos.join("IMG_1268 2.JPG.supplemental-metadata.json"),
            r#"{"title": "IMG_1268 2.JPG"}"#,
        );

        let quarantena = temp.path().join("quarantena");
        let report = clean(
            &photos,
            &CleanOptions {
                mode: CleanMode::Quarantine,
                destination: Some(quarantena.clone()),
                ..Default::default()
            },
            &no_progress,
        )
        .expect("quarantena");

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.duplicates_handled, 1);
        assert_eq!(report.companions_handled, 1, "the sidecar has to follow");

        // The original survives, with its sidecar.
        assert!(photos.join("IMG_1268.JPG").is_file());
        assert!(photos
            .join("IMG_1268.JPG.supplemental-metadata.json")
            .is_file());

        // The copy left along with its own sidecar: no orphans.
        assert!(!photos.join("IMG_1268 2.JPG").exists());
        assert!(!photos
            .join("IMG_1268 2.JPG.supplemental-metadata.json")
            .exists());

        // And we go all the way back.
        let manifest = report.manifest.expect("registro");
        let restore = restore_quarantine(&manifest).expect("restored");
        assert_eq!(restore.restored, 2);
        assert!(photos.join("IMG_1268 2.JPG").is_file());
        assert!(photos
            .join("IMG_1268 2.JPG.supplemental-metadata.json")
            .is_file());
    }

    /// Two sidecars can have identical content while belonging to different
    /// photos. Treating them as independent duplicates would remove one, and the
    /// photo left without would lose date and coordinates: damage that, looking
    /// at the surviving files, is not even visible.
    #[test]
    fn sidecars_are_not_deduplicated_against_each_other() {
        let temp = TempDir::new("sidecar-dedup");
        let photos = temp.path().join("Google Foto");

        // Two different photos, with sidecars of identical content.
        write_file(&photos.join("IMG_1.JPG"), "pixel della prima");
        write_file(&photos.join("IMG_2.JPG"), "pixel della second");
        write_file(&photos.join("IMG_1.JPG.json"), r#"{"t": "1577880000"}"#);
        write_file(&photos.join("IMG_2.JPG.json"), r#"{"t": "1577880000"}"#);

        let plan =
            plan_clean(&photos, &CleanOptions::default(), usize::MAX, &no_progress).expect("plan");

        assert_eq!(
            plan.duplicate_copies, 0,
            "identical sidecars are not duplicates to remove"
        );
        assert_eq!(plan.files_scanned, 4);
        assert_eq!(plan.files_kept, 4, "everything stays");
    }

    #[test]
    fn the_companion_prefix_does_not_confuse_similar_names() {
        let temp = TempDir::new("drive-prefix");
        let photos = temp.path().join("f");
        write_file(&photos.join("IMG_1268.JPG"), "a");
        write_file(&photos.join("IMG_1268.JPG.json"), "sidecar del first");
        write_file(&photos.join("IMG_1268 2.JPG"), "b");
        write_file(&photos.join("IMG_1268 2.JPG.json"), "sidecar del secondo");

        // The index is the one `plan_clean` builds during the scan.
        let mut index: DirIndex = HashMap::new();
        index.insert(
            photos.clone(),
            std::fs::read_dir(&photos)
                .expect("lettura folder")
                .flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .collect(),
        );

        // `IMG_1268.JPG` must not lay claim to the files of `IMG_1268 2.JPG`.
        let compagni = find_companions(&photos.join("IMG_1268.JPG"), &index);
        assert_eq!(compagni.len(), 1);
        assert!(compagni[0].ends_with("IMG_1268.JPG.json"));
    }

    #[test]
    fn refuses_a_destination_inside_the_source() {
        let temp = TempDir::new("drive-ricorsione");
        let drive = build_drive(temp.path());

        let outcome = clean(
            &drive,
            &CleanOptions {
                mode: CleanMode::Quarantine,
                destination: Some(drive.join("quarantena")),
                ..Default::default()
            },
            &no_progress,
        );

        assert!(outcome.is_err(), "a nested destination has to be refused");
    }

    #[test]
    fn recognises_google_placeholders() {
        for ext in ["gdoc", "gsheet", "gslides", "gform"] {
            let path = PathBuf::from(format!("appunti.{ext}"));
            assert_eq!(
                FileCategory::from_path(&path),
                FileCategory::Placeholder,
                "{ext} has to be a placeholder"
            );
        }
    }
}
