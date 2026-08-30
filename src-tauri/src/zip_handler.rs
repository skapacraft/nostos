// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Inspection and extraction of `takeout-*.zip` archives.
//!
//! Google delivers a Takeout as a series of numbered ZIPs. Here the archive is
//! read as a stream: no temporary copy, no implicit extraction. Extraction
//! happens only on an explicit request and into a destination chosen by the
//! user.

use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app_state::{Result, TakeoutError};

/// One entry of an archive, without its content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub compressed_size: u64,
}

/// Summary of an archive inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSummary {
    pub path: PathBuf,
    pub entry_count: usize,
    pub file_count: usize,
    pub uncompressed_bytes: u64,
    pub compressed_bytes: u64,
    /// Top-level folders, typically just `Takeout/`.
    pub top_level: Vec<String>,
    /// Entries rejected because their path was unsafe.
    pub rejected: Vec<String>,
}

/// Outcome of an extraction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractReport {
    pub destination: PathBuf,
    /// Archives actually processed, in numbering order.
    pub archives: Vec<PathBuf>,
    pub files_written: usize,
    pub dirs_created: usize,
    pub bytes_written: u64,
    /// Entries discarded because their path was unsafe.
    pub skipped: Vec<String>,
    /// Paths present in more than one archive of the series.
    pub collisions: Vec<String>,
}

/// The series of archives making up a single Takeout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSeries {
    /// Common prefix, without the sequence number.
    pub prefix: String,
    pub archives: Vec<PathBuf>,
    /// Numbers missing from the sequence, if any.
    pub missing: Vec<u32>,
    pub total_compressed_bytes: u64,
}

/// Recognises a Takeout archive from its filename.
pub fn is_takeout_archive(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip") && (lower.starts_with("takeout") || lower.contains("takeout"))
}

/// Opens the archive read-only.
fn open_archive(path: &Path) -> Result<zip::ZipArchive<File>> {
    let file = File::open(path).map_err(|e| TakeoutError::io(path, e))?;
    zip::ZipArchive::new(file).map_err(|e| TakeoutError::Archive(format!("{path:?}: {e}")))
}

/// Lists the contents without extracting anything.
pub fn inspect(path: &Path) -> Result<ArchiveSummary> {
    let mut archive = open_archive(path)?;

    let mut summary = ArchiveSummary {
        path: path.to_path_buf(),
        entry_count: archive.len(),
        file_count: 0,
        uncompressed_bytes: 0,
        compressed_bytes: 0,
        top_level: Vec::new(),
        rejected: Vec::new(),
    };

    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| TakeoutError::Archive(format!("voce {index}: {e}")))?;
        let name = entry.name().to_string();

        if safe_relative_path(&name).is_none() {
            summary.rejected.push(name);
            continue;
        }

        if let Some(first) = name.split('/').next().filter(|s| !s.is_empty()) {
            let first = first.to_string();
            if !summary.top_level.contains(&first) {
                summary.top_level.push(first);
            }
        }

        if !entry.is_dir() {
            summary.file_count += 1;
            summary.uncompressed_bytes += entry.size();
            summary.compressed_bytes += entry.compressed_size();
        }
    }

    summary.top_level.sort();
    Ok(summary)
}

/// Returns the first `limit` entries, for the preview in the UI.
pub fn list_entries(path: &Path, limit: usize) -> Result<Vec<ArchiveEntry>> {
    let mut archive = open_archive(path)?;
    let mut entries = Vec::new();

    for index in 0..archive.len().min(limit) {
        let entry = archive
            .by_index(index)
            .map_err(|e| TakeoutError::Archive(format!("voce {index}: {e}")))?;
        entries.push(ArchiveEntry {
            name: entry.name().to_string(),
            is_dir: entry.is_dir(),
            size: entry.size(),
            compressed_size: entry.compressed_size(),
        });
    }

    Ok(entries)
}

/// Splits an archive name into series prefix and sequence number.
///
/// Google numbers exports as `takeout-20260805T090000Z-001.zip`. Each archive
/// is self-contained and holds a slice of the tree: this is not one archive
/// split into volumes, so each opens and reads on its own.
fn series_key(path: &Path) -> Option<(String, u32)> {
    let stem = path.file_stem()?.to_str()?;
    let dash = stem.rfind('-')?;
    let (prefix, rest) = stem.split_at(dash);
    let digits = rest.strip_prefix('-')?;

    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if prefix.is_empty() {
        return None;
    }

    Some((prefix.to_string(), digits.parse().ok()?))
}

/// Finds every archive of the same series starting from any one of them.
///
/// If the name does not follow the numbered scheme, the series contains only
/// the file given: a small Takeout fits in a single archive.
pub fn discover_series(path: &Path) -> Result<ArchiveSeries> {
    crate::app_state::require_existing(path)?;

    let Some((prefix, _)) = series_key(path) else {
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        return Ok(ArchiveSeries {
            prefix: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string(),
            archives: vec![path.to_path_buf()],
            missing: Vec::new(),
            total_compressed_bytes: size,
        });
    };

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let entries = fs::read_dir(parent).map_err(|e| TakeoutError::io(parent, e))?;

    let mut found: Vec<(u32, PathBuf)> = Vec::new();
    let mut total = 0u64;

    for entry in entries.flatten() {
        let candidate = entry.path();
        if !candidate.is_file() {
            continue;
        }
        let is_zip = candidate
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("zip"))
            .unwrap_or(false);
        if !is_zip {
            continue;
        }

        if let Some((candidate_prefix, number)) = series_key(&candidate) {
            if candidate_prefix == prefix {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                found.push((number, candidate));
            }
        }
    }

    found.sort_by_key(|(number, _)| *number);

    // A missing number usually means an interrupted download. Say so before
    // extracting, not after discovering half the photos are gone.
    let mut missing = Vec::new();
    if let (Some((first, _)), Some((last, _))) = (found.first(), found.last()) {
        let present: std::collections::HashSet<u32> = found.iter().map(|(n, _)| *n).collect();
        for number in *first..=*last {
            if !present.contains(&number) {
                missing.push(number);
            }
        }
    }

    Ok(ArchiveSeries {
        prefix,
        archives: found.into_iter().map(|(_, path)| path).collect(),
        missing,
        total_compressed_bytes: total,
    })
}

/// Extracts a single archive into `destination`.
///
/// Every path is normalised and checked: absolute entries, entries with `..`
/// and entries with a volume prefix are discarded rather than written outside
/// the destination (zip-slip, CVE-2018-1000544 and friends).
pub fn extract(path: &Path, destination: &Path) -> Result<ExtractReport> {
    extract_series(
        std::slice::from_ref(&path.to_path_buf()),
        destination,
        &crate::app_state::no_progress,
    )
}

/// Extracts an entire series of archives into one destination tree.
///
/// The archives are merged: folders overlap without conflict, while a file
/// already written by an earlier archive is not overwritten but recorded as a
/// collision. In an intact Takeout collisions are zero, so finding one
/// signals a problem with the download.
pub fn extract_series(
    archives: &[PathBuf],
    destination: &Path,
    progress: crate::app_state::ProgressSink<'_>,
) -> Result<ExtractReport> {
    use crate::app_state::{Phase, Progress};

    fs::create_dir_all(destination).map_err(|e| TakeoutError::io(destination, e))?;
    let dest_root = destination
        .canonicalize()
        .map_err(|e| TakeoutError::io(destination, e))?;

    // Preliminary count, so the progress bar has a real total.
    let mut total_entries = 0usize;
    for archive_path in archives {
        total_entries += open_archive(archive_path)?.len();
    }
    progress(Progress::new(Phase::Scanning, 0, total_entries, 0));

    let mut report = ExtractReport {
        destination: dest_root.clone(),
        archives: archives.to_vec(),
        ..Default::default()
    };

    let mut written_files: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut done = 0usize;
    // Google wraps the whole export in one directory (`Takeout/`, here
    // `sample-takeout/`): the folder worth handing back to the caller is the
    // one actually holding the sections, not the pick destination itself.
    let mut common_top: Option<std::ffi::OsString> = None;
    let mut single_top_level = true;

    for archive_path in archives {
        let mut archive = open_archive(archive_path)?;

        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|e| {
                TakeoutError::Archive(format!("{archive_path:?} voce {index}: {e}"))
            })?;
            let raw_name = entry.name().to_string();

            done += 1;

            let Some(relative) = safe_relative_path(&raw_name) else {
                report.skipped.push(raw_name);
                continue;
            };

            // AppleDouble junk from zipping on a Mac: resource forks with
            // nothing Takeout ever put there, and the reason a real export,
            // zipped and re-shared through Finder, would otherwise show up
            // as two top-level folders instead of one.
            if matches!(relative.components().next(), Some(Component::Normal(first)) if first == "__MACOSX")
                || relative.file_name().is_some_and(|name| name == ".DS_Store")
            {
                continue;
            }

            let target = dest_root.join(&relative);
            // Second barrier: even after normalisation the target must stay
            // under the destination root.
            if !target.starts_with(&dest_root) {
                return Err(TakeoutError::UnsafeEntry(raw_name));
            }

            if single_top_level {
                match relative.components().next() {
                    Some(Component::Normal(first)) => match &common_top {
                        None => common_top = Some(first.to_os_string()),
                        Some(top) if top == first => {}
                        Some(_) => single_top_level = false,
                    },
                    _ => single_top_level = false,
                }
            }

            if entry.is_dir() {
                fs::create_dir_all(&target).map_err(|e| TakeoutError::io(&target, e))?;
                report.dirs_created += 1;
                continue;
            }

            if !written_files.insert(target.clone()) {
                report.collisions.push(raw_name);
                continue;
            }

            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| TakeoutError::io(parent, e))?;
            }

            let mut out = File::create(&target).map_err(|e| TakeoutError::io(&target, e))?;
            // `io::copy` works in blocks: a 4 GB file never enters memory.
            let written =
                io::copy(&mut entry, &mut out).map_err(|e| TakeoutError::io(&target, e))?;

            report.files_written += 1;
            report.bytes_written += written;

            progress(
                Progress::new(
                    Phase::Extracting,
                    done,
                    total_entries,
                    report.skipped.len() + report.collisions.len(),
                )
                .with_current(relative.to_string_lossy()),
            );
        }
    }

    progress(Progress::new(
        Phase::Done,
        total_entries,
        total_entries,
        report.skipped.len() + report.collisions.len(),
    ));

    if single_top_level {
        if let Some(top) = common_top {
            let inner = dest_root.join(&top);
            if inner.is_dir() {
                report.destination = inner;
            }
        }
    }

    Ok(report)
}

/// Normalises a ZIP entry name into a safe relative path.
///
/// Returns `None` if the entry tries to escape the destination.
fn safe_relative_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('\0') {
        return None;
    }
    // ZIPs use `/` as separator even when produced on Windows, but some tools
    // write `\`: normalise before parsing.
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') {
        return None;
    }

    let mut out = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_relative_paths() {
        assert_eq!(
            safe_relative_path("Takeout/Google Foto/IMG_1.jpg"),
            Some(PathBuf::from("Takeout/Google Foto/IMG_1.jpg"))
        );
        assert_eq!(
            safe_relative_path("./Takeout/archive_browser.html"),
            Some(PathBuf::from("Takeout/archive_browser.html"))
        );
    }

    #[test]
    fn refuses_zip_slip_attempts() {
        assert_eq!(safe_relative_path("../../etc/passwd"), None);
        assert_eq!(safe_relative_path("/etc/passwd"), None);
        assert_eq!(safe_relative_path("Takeout/../../fuori.txt"), None);
        assert_eq!(safe_relative_path("..\\..\\Windows\\System32"), None);
        assert_eq!(safe_relative_path(""), None);
    }

    #[test]
    fn recognises_takeout_archive_names() {
        assert!(is_takeout_archive(Path::new(
            "takeout-20260805T090000Z-001.zip"
        )));
        assert!(is_takeout_archive(Path::new("/tmp/Takeout.zip")));
        assert!(!is_takeout_archive(Path::new("photos.zip")));
        assert!(!is_takeout_archive(Path::new("takeout.tgz")));
    }

    #[test]
    fn splits_the_name_into_series_and_number() {
        assert_eq!(
            series_key(Path::new("/t/takeout-20260805T090000Z-001.zip")),
            Some(("takeout-20260805T090000Z".to_string(), 1))
        );
        assert_eq!(
            series_key(Path::new("/t/takeout-20260805T090000Z-012.zip")),
            Some(("takeout-20260805T090000Z".to_string(), 12))
        );
        // No sequence number means no series: this is a standalone archive.
        assert_eq!(series_key(Path::new("/t/Takeout.zip")), None);
        assert_eq!(series_key(Path::new("/t/takeout-finale.zip")), None);
    }

    /// Builds an archive with the given entries, for the extraction tests.
    fn build_archive(path: &Path, entries: &[(&str, &str)]) {
        let file = File::create(path).expect("creazione archivio");
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        for (name, content) in entries {
            if name.ends_with('/') {
                writer.add_directory(*name, options).expect("folder");
            } else {
                writer.start_file(*name, options).expect("voce");
                std::io::Write::write_all(&mut writer, content.as_bytes()).expect("contenuto");
            }
        }

        writer.finish().expect("chiusura archivio");
    }

    #[test]
    fn merges_the_archives_of_a_series_into_one_tree() {
        let temp = crate::app_state::testing::TempDir::new("series_path");
        let dir = temp.path();

        // Google splits the export into self-contained archives: the `Takeout/`
        // folder and the section subfolders reappear in every one of them.
        build_archive(
            &dir.join("takeout-20260805T090000Z-001.zip"),
            &[
                ("Takeout/", ""),
                ("Takeout/Google Foto/", ""),
                ("Takeout/Google Foto/IMG_0001.JPG", "first"),
            ],
        );
        build_archive(
            &dir.join("takeout-20260805T090000Z-002.zip"),
            &[
                ("Takeout/", ""),
                ("Takeout/Google Foto/", ""),
                ("Takeout/Google Foto/IMG_0002.JPG", "secondo"),
                ("Takeout/Drive/relazione.docx", "terzo"),
            ],
        );
        // An archive from a different export must not join the series.
        build_archive(
            &dir.join("takeout-20250101T000000Z-001.zip"),
            &[("x.txt", "estraneo")],
        );

        let series = discover_series(&dir.join("takeout-20260805T090000Z-002.zip"))
            .expect("individuazione series_path");
        assert_eq!(
            series.archives.len(),
            2,
            "only the archives of the same export"
        );
        assert!(series.missing.is_empty());
        assert!(series.archives[0].ends_with("takeout-20260805T090000Z-001.zip"));

        let dest = dir.join("extracted");
        let report = extract_series(&series.archives, &dest, &crate::app_state::no_progress)
            .expect("series extraction");

        assert_eq!(report.files_written, 3);
        assert!(report.skipped.is_empty());
        assert!(
            report.collisions.is_empty(),
            "repeated folders are not collisions"
        );

        assert_eq!(
            std::fs::read_to_string(dest.join("Takeout/Google Foto/IMG_0001.JPG")).unwrap(),
            "first"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("Takeout/Google Foto/IMG_0002.JPG")).unwrap(),
            "secondo"
        );
        assert!(dest.join("Takeout/Drive/relazione.docx").is_file());
        // Google wraps every export in one folder: callers that load the
        // returned destination straight away must land inside it, not beside
        // it, or every section looks unrecognised.
        assert_eq!(
            report.destination,
            dest.join("Takeout").canonicalize().unwrap()
        );
    }

    #[test]
    fn ignores_macos_zip_junk_when_finding_the_export_root() {
        let temp = crate::app_state::testing::TempDir::new("macosx_junk");
        let dir = temp.path();

        // Finder's "Compress" adds a resource-fork mirror beside the real
        // content: two top-level entries in the archive, but only one of
        // them is the export.
        build_archive(
            &dir.join("Takeout.zip"),
            &[
                ("Takeout/Contatti/Contacts.vcf", "x"),
                ("__MACOSX/Takeout/Contatti/._Contacts.vcf", "junk"),
                ("Takeout/.DS_Store", "junk"),
            ],
        );

        let dest = dir.join("extracted");
        let report = extract_series(
            &[dir.join("Takeout.zip")],
            &dest,
            &crate::app_state::no_progress,
        )
        .expect("extraction");

        assert_eq!(report.files_written, 1, "only the real file, junk skipped");
        assert_eq!(report.destination, dest.join("Takeout").canonicalize().unwrap());
        assert!(!dest.join("__MACOSX").exists());
    }

    #[test]
    fn flags_the_numbers_missing_from_the_series() {
        let temp = crate::app_state::testing::TempDir::new("buchi");
        let dir = temp.path();

        for number in ["001", "003"] {
            build_archive(
                &dir.join(format!("takeout-20260805T090000Z-{number}.zip")),
                &[("Takeout/nota.txt", "x")],
            );
        }

        let series =
            discover_series(&dir.join("takeout-20260805T090000Z-001.zip")).expect("series_path");
        assert_eq!(series.archives.len(), 2);
        assert_eq!(series.missing, vec![2], "the download is incomplete");
    }

    #[test]
    fn records_collisions_without_overwriting() {
        let temp = crate::app_state::testing::TempDir::new("collisioni");
        let dir = temp.path();

        build_archive(
            &dir.join("takeout-series_path-001.zip"),
            &[("Takeout/doppio.txt", "originale")],
        );
        build_archive(
            &dir.join("takeout-series_path-002.zip"),
            &[("Takeout/doppio.txt", "sovrascrittura")],
        );

        let series =
            discover_series(&dir.join("takeout-series_path-001.zip")).expect("series_path");
        let dest = dir.join("extracted");
        let report = extract_series(&series.archives, &dest, &crate::app_state::no_progress)
            .expect("estrazione");

        assert_eq!(report.files_written, 1);
        assert_eq!(report.collisions.len(), 1);
        // First one wins: the second must not be able to rewrite the content.
        assert_eq!(
            std::fs::read_to_string(dest.join("Takeout/doppio.txt")).unwrap(),
            "originale"
        );
    }

    #[test]
    fn reports_progress_during_extraction() {
        use crate::app_state::{Phase, Progress};
        use std::sync::Mutex;

        let temp = crate::app_state::testing::TempDir::new("avanzamento");
        let dir = temp.path();
        build_archive(
            &dir.join("takeout-p-001.zip"),
            &[("Takeout/a.txt", "a"), ("Takeout/b.txt", "b")],
        );

        let events: Mutex<Vec<Progress>> = Mutex::new(Vec::new());
        let sink = |p: Progress| events.lock().unwrap().push(p);

        extract_series(
            &[dir.join("takeout-p-001.zip")],
            &dir.join("extracted"),
            &sink,
        )
        .expect("estrazione");

        let events = events.into_inner().unwrap();
        assert!(events.len() >= 3, "scanning, file, completion");
        assert_eq!(events.first().unwrap().phase, Phase::Scanning);

        let ultimo = events.last().unwrap();
        assert_eq!(ultimo.phase, Phase::Done);
        assert_eq!(ultimo.done, ultimo.total);
    }
}
