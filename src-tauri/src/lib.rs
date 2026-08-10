// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Nostos: local processing of Google Takeout exports.
//!
//! The architectural rule of this project: no module opens network connections.
//! There is no HTTP client in the dependency graph, no telemetry, no
//! auto-updater. The data read stays on the user's disk and in memory for the
//! duration of the session.

mod albums;
mod app_state;
mod calendar;
mod contacts;
mod drive;
mod exif_parser;
mod zip_handler;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, State};
use walkdir::WalkDir;

use albums::AlbumIndex;
use app_state::{
    trace_dev, AppInfo, AppState, ExportReport, FolderSize, LoadedSource, Notice, Phase,
    Preferences, PrivacyReport, Progress, Result, SectionSummary, SourceKind, SourceSummary,
    SpaceEstimate, TakeoutError, TakeoutSection,
};
use calendar::CalendarReport;
use contacts::ContactsReport;
use drive::{CleanOptions, CleanPlan, CleanReport, DriveReport, RestoreReport, SidecarSweepReport};
use exif_parser::{PhotoScanReport, RepairReport, WriteOptions};
use zip_handler::{ArchiveEntry, ArchiveSeries, ArchiveSummary, ExtractReport};

/// How many sample records to return to the frontend for previews.
const SAMPLE_SIZE: usize = 25;
/// Cap on long lists (duplicates, placeholders, largest files).
const MAX_ITEMS: usize = 50;

/// Name of the progress event the frontend listens to.
const PROGRESS_EVENT: &str = "takeout://progress";

/// Name shown to the user, distinct from the Cargo package name.
const APP_NAME: &str = "Nostos";

/// Identifier of the menu item that opens the guide.
const MENU_HELP_ID: &str = "guida";

/// Identifier of the menu item that opens the problem report.
const MENU_REPORT_ID: &str = "oth-report";

/// Identifier of the menu item that opens version and updates.
const MENU_VERSION_ID: &str = "oth-version";

/// Event with which the menu asks the frontend to show the problem report.
const SHOW_REPORT_EVENT: &str = "takeout://mostra-segnalazione";

/// Event with which the menu asks the frontend to show version and updates.
const SHOW_VERSION_EVENT: &str = "takeout://mostra-versione";

/// Event with which the menu asks the frontend to show the guide.
const SHOW_HELP_EVENT: &str = "takeout://mostra-guida";

/// Builds the menu bar.
///
/// Tauri could generate a default one, but the macOS "About" and "Hide" items
/// would take their name from the process, which during development is the name
/// of the executable (`nostos`) because there is no `.app` bundle with
/// its `CFBundleName` yet. Declaring them by hand keeps the name right both in
/// development and in the distributed package.
fn build_menu<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};

    let about = AboutMetadata {
        name: Some(APP_NAME.to_string()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        authors: Some(vec![env!("CARGO_PKG_AUTHORS").to_string()]),
        copyright: Some("Copyright (C) 2026 SkapaCraft".to_string()),
        license: Some("GPL-3.0-or-later".to_string()),
        // The website stays in the comments as text. The `website` field becomes a
        // clickable link in the system window on some platforms, and this
        // application does not open addresses.
        comments: Some(format!(
            "Processes your Google Takeout exports locally.\n{}",
            env!("CARGO_PKG_HOMEPAGE")
        )),
        ..Default::default()
    };

    let guide = MenuItem::with_id(app, MENU_HELP_ID, "Nostos guide", true, None::<&str>)?;

    // Distinct from the system "About": that window states who wrote the
    // program, this item says how old the copy in front of you is and where
    // newer ones appear. Neither asks anything of a server.
    let version = MenuItem::with_id(
        app,
        MENU_VERSION_ID,
        "Version and updates",
        true,
        None::<&str>,
    )?;

    let report = MenuItem::with_id(
        app,
        MENU_REPORT_ID,
        "Report a problem...",
        true,
        None::<&str>,
    )?;

    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("Undo"))?,
            &PredefinedMenuItem::redo(app, Some("Redo"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("Cut"))?,
            &PredefinedMenuItem::copy(app, Some("Copy"))?,
            &PredefinedMenuItem::paste(app, Some("Paste"))?,
            &PredefinedMenuItem::select_all(app, Some("Select All"))?,
        ],
    )?;

    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("Minimise"))?,
            &PredefinedMenuItem::fullscreen(app, Some("Full Screen"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, Some("Close Window"))?,
        ],
    )?;

    let help = Submenu::with_items(app, "Help", true, &[&guide, &version, &report])?;

    #[cfg(target_os = "macos")]
    {
        let application = Submenu::with_items(
            app,
            APP_NAME,
            true,
            &[
                &PredefinedMenuItem::about(app, Some(&format!("About {APP_NAME}")), Some(about))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, Some("Services"))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, Some(&format!("Hide {APP_NAME}")))?,
                &PredefinedMenuItem::hide_others(app, Some("Hide Others"))?,
                &PredefinedMenuItem::show_all(app, Some("Show All"))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, Some(&format!("Quit {APP_NAME}")))?,
            ],
        )?;
        Menu::with_items(app, &[&application, &edit, &window, &help])
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Outside macOS there is no application menu: "About" and "Quit" go under
        // File, where users look for them.
        let file = Submenu::with_items(
            app,
            "File",
            true,
            &[
                &PredefinedMenuItem::about(app, Some(&format!("About {APP_NAME}")), Some(about))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, Some("Quit"))?,
            ],
        )?;
        Menu::with_items(app, &[&file, &edit, &window, &help])
    }
}

/// Minimum interval between two progress events.
///
/// Emitting one per file would mean tens of thousands of IPC messages and as
/// many React renders: the window stutters precisely while showing that it is
/// working. At 80 ms the eye sees continuous progress and the rendering thread
/// stays free.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(80);

/// Builds the sink forwarding progress to the webview, with throttling.
///
/// The start and end events always get through: they are the ones that make the
/// bar appear and disappear, and losing them would leave the UI in a wrong state.
fn progress_emitter(app: AppHandle) -> impl Fn(Progress) + Send + Sync {
    let last = Mutex::new(None::<Instant>);

    move |progress: Progress| {
        let always = matches!(progress.phase, Phase::Scanning | Phase::Done);

        let Ok(mut guard) = last.lock() else {
            return;
        };
        let due = guard.is_none_or(|instant| instant.elapsed() >= PROGRESS_INTERVAL);
        if !always && !due {
            return;
        }
        *guard = Some(Instant::now());
        drop(guard);

        // An emission error means the window is closed: processing can carry on
        // and finish by itself, there is no case for aborting it.
        let _ = app.emit(PROGRESS_EVENT, &progress);
    }
}

/// Runs blocking work outside the async runtime, so the UI does not freeze.
async fn in_background<T, F>(work: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| TakeoutError::Task(e.to_string()))?
}

// ---------------------------------------------------------------------------
// Source recognition
// ---------------------------------------------------------------------------

/// Locates the real root of the Takeout.
///
/// The user can drag the `Takeout/` folder or the folder containing it: we
/// normalise both cases.
fn resolve_takeout_root(path: &Path) -> PathBuf {
    let nested = path.join("Takeout");
    if nested.is_dir() {
        return nested;
    }
    path.to_path_buf()
}

/// Counts files and bytes of a subfolder.
fn measure_dir(path: &Path) -> (usize, u64) {
    let mut files = 0usize;
    let mut bytes = 0u64;

    for entry in WalkDir::new(path).follow_links(false).into_iter().flatten() {
        if entry.file_type().is_file() {
            files += 1;
            bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }

    (files, bytes)
}

/// Analyses an already extracted Takeout folder.
fn analyze_folder(path: &Path) -> Result<SourceSummary> {
    app_state::require_existing(path)?;
    let root = resolve_takeout_root(path);

    let mut summary = SourceSummary {
        display_name: root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Takeout")
            .to_string(),
        root: root.clone(),
        kind: SourceKind::Folder,
        sections: Vec::new(),
        file_count: 0,
        total_bytes: 0,
        warnings: Vec::new(),
    };

    let entries = std::fs::read_dir(&root).map_err(|e| TakeoutError::io(&root, e))?;

    for entry in entries.flatten() {
        let child = entry.path();
        if !child.is_dir() {
            continue;
        }
        let dir_name = child
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        // Hidden folders are not part of the export.
        if dir_name.starts_with('.') {
            continue;
        }

        let section = TakeoutSection::from_dir_name(&dir_name);
        let (file_count, total_bytes) = measure_dir(&child);

        summary.file_count += file_count;
        summary.total_bytes += total_bytes;
        summary.sections.push(SectionSummary {
            section,
            dir_name,
            path: child,
            file_count,
            total_bytes,
        });
    }

    summary
        .sections
        .sort_by_key(|section| std::cmp::Reverse(section.total_bytes));

    if summary.sections.is_empty() {
        summary.warnings.push(Notice::NoSectionsFound);
    }

    Ok(summary)
}

/// Analyses a `takeout-*.zip` archive without extracting it.
fn analyze_archive(path: &Path) -> Result<SourceSummary> {
    app_state::require_existing(path)?;
    let archive = zip_handler::inspect(path)?;

    let mut summary = SourceSummary {
        display_name: path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("archivio")
            .to_string(),
        root: path.to_path_buf(),
        kind: SourceKind::Archive,
        sections: Vec::new(),
        file_count: archive.file_count,
        total_bytes: archive.uncompressed_bytes,
        warnings: Vec::new(),
    };

    // Inside the archive the sections are the second-level folders, under
    // `Takeout/`. We list them without extracting, so without per-section sizes:
    // those would require a second complete pass.
    let mut section_names: Vec<String> = Vec::new();
    for entry in zip_handler::list_entries(path, usize::MAX)? {
        let parts: Vec<&str> = entry.name.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() < 2 {
            continue;
        }
        let candidate = parts[1].to_string();
        if !section_names.contains(&candidate) {
            section_names.push(candidate);
        }
    }

    for dir_name in section_names {
        let section = TakeoutSection::from_dir_name(&dir_name);
        summary.sections.push(SectionSummary {
            section,
            path: path.join(&dir_name),
            dir_name,
            file_count: 0,
            total_bytes: 0,
        });
    }

    if !archive.rejected.is_empty() {
        summary.warnings.push(Notice::UnsafeArchiveEntries {
            count: archive.rejected.len(),
        });
    }
    summary.warnings.push(Notice::ArchiveNotExtracted);

    Ok(summary)
}

// ---------------------------------------------------------------------------
// Commands exposed to the frontend
// ---------------------------------------------------------------------------

/// Loads a source (folder or archive) and returns its summary.
#[tauri::command]
fn load_source(path: String, state: State<'_, AppState>) -> Result<SourceSummary> {
    let path = PathBuf::from(path);
    app_state::require_existing(&path)?;

    let summary = if path.is_dir() {
        analyze_folder(&path)?
    } else if zip_handler::is_takeout_archive(&path) {
        analyze_archive(&path)?
    } else {
        return Err(TakeoutError::UnrecognisedSource(path));
    };

    state.set_source(LoadedSource {
        root: summary.root.clone(),
        summary: summary.clone(),
    })?;

    Ok(summary)
}

/// Resolves the path to work on.
///
/// The analysis commands accept an explicit path (a section chosen in the UI)
/// or no path at all, in which case they work on the whole loaded source.
/// source caricata.
fn target_path(path: Option<String>, state: &State<'_, AppState>) -> Result<PathBuf> {
    match path {
        Some(path) => Ok(PathBuf::from(path)),
        None => state.root(),
    }
}

/// Summary of the currently loaded source.
#[tauri::command]
fn current_source(state: State<'_, AppState>) -> Result<SourceSummary> {
    state.summary()
}

/// Forgets the current source.
#[tauri::command]
fn close_source(state: State<'_, AppState>) -> Result<()> {
    state.clear()
}

/// Inspects an archive without extracting it.
#[tauri::command]
fn inspect_archive(path: String) -> Result<ArchiveSummary> {
    zip_handler::inspect(Path::new(&path))
}

/// Lists the first entries of an archive.
#[tauri::command]
fn list_archive_entries(path: String, limit: Option<usize>) -> Result<Vec<ArchiveEntry>> {
    zip_handler::list_entries(Path::new(&path), limit.unwrap_or(MAX_ITEMS))
}

/// Extracts an archive into the destination the user indicated.
#[tauri::command]
fn extract_archive(path: String, destination: String) -> Result<ExtractReport> {
    zip_handler::extract(Path::new(&path), Path::new(&destination))
}

/// Finds every archive making up the same export.
#[tauri::command]
fn discover_archive_series(path: String) -> Result<ArchiveSeries> {
    zip_handler::discover_series(Path::new(&path))
}

/// Extracts the entire series of archives into a single tree.
///
/// This is the command the interface actually uses: starting from any archive it
/// reconstructs the series and merges it, instead of forcing the user to extract
/// twelve files by hand one on top of the other.
#[tauri::command]
async fn extract_takeout(
    app: AppHandle,
    path: String,
    destination: String,
) -> Result<ExtractReport> {
    in_background(move || {
        let series = zip_handler::discover_series(Path::new(&path))?;
        let sink = progress_emitter(app);
        zip_handler::extract_series(&series.archives, Path::new(&destination), &sink)
    })
    .await
}

/// Analyses a Google Photos folder.
#[tauri::command]
fn scan_photos(path: Option<String>, state: State<'_, AppState>) -> Result<PhotoScanReport> {
    exif_parser::scan_directory(&target_path(path, &state)?, SAMPLE_SIZE)
}

/// Repairs date and coordinates of the media, in the mode requested.
///
/// The command is asynchronous and delegates to a worker thread: on tens of
/// thousands of photos a synchronous command would block the window for minutes.
#[tauri::command]
async fn repair_photos(
    app: AppHandle,
    path: String,
    options: WriteOptions,
) -> Result<RepairReport> {
    in_background(move || {
        let sink = progress_emitter(app);
        exif_parser::apply_metadata(Path::new(&path), &options, &sink)
    })
    .await
}

/// Analyses the Contacts export.
#[tauri::command]
fn scan_contacts(path: Option<String>, state: State<'_, AppState>) -> Result<ContactsReport> {
    contacts::scan_directory(&target_path(path, &state)?, SAMPLE_SIZE)
}

/// Analyses the Drive export.
#[tauri::command]
fn scan_drive(path: Option<String>, state: State<'_, AppState>) -> Result<DriveReport> {
    drive::scan_directory(&target_path(path, &state)?, MAX_ITEMS)
}

/// Computes the cleanup plan without touching anything.
///
/// Deduplication reads the content of the candidate files, so on a large export
/// it is a long operation: it goes in the background like the others.
#[tauri::command]
async fn plan_drive_clean(
    app: AppHandle,
    path: String,
    options: CleanOptions,
) -> Result<CleanPlan> {
    in_background(move || {
        let sink = progress_emitter(app);
        drive::plan_clean(Path::new(&path), &options, MAX_ITEMS, &sink)
    })
    .await
}

/// Performs the cleanup: a clean tree elsewhere, or reversible quarantine.
#[tauri::command]
async fn clean_drive(app: AppHandle, path: String, options: CleanOptions) -> Result<CleanReport> {
    in_background(move || {
        let sink = progress_emitter(app);
        drive::clean(Path::new(&path), &options, &sink)
    })
    .await
}

/// Moves the sidecars whose content is now inside the files.
///
/// The last step of a successful repair, not a cleanup: it moves only the JSONs
/// that are no longer the sole copy of anything, and writes the ledger that
/// allows undoing it.
#[tauri::command]
async fn sweep_sidecars(
    app: AppHandle,
    path: String,
    destination: String,
) -> Result<SidecarSweepReport> {
    in_background(move || {
        let sink = progress_emitter(app);
        drive::sweep_applied_sidecars(Path::new(&path), Path::new(&destination), MAX_ITEMS, &sink)
    })
    .await
}

/// Puts the files moved to quarantine back where they were.
#[tauri::command]
async fn restore_quarantine(manifest: String) -> Result<RestoreReport> {
    in_background(move || drive::restore_quarantine(Path::new(&manifest))).await
}

/// Reconstructs the structure of a Google Photos export: albums, year folders
/// and edited versions.
#[tauri::command]
async fn scan_albums(path: String) -> Result<AlbumIndex> {
    in_background(move || albums::build_index(Path::new(&path), MAX_ITEMS)).await
}

/// Writes the album manifest, to be done before deduplicating.
#[tauri::command]
fn export_album_manifest(path: String, destination: String) -> Result<ExportReport> {
    albums::export_manifest(Path::new(&path), Path::new(&destination))
}

/// Analyses the Calendar export.
#[tauri::command]
fn scan_calendar(path: Option<String>, state: State<'_, AppState>) -> Result<CalendarReport> {
    calendar::scan_directory(&target_path(path, &state)?, SAMPLE_SIZE)
}

/// Writes a clean, deduplicated vCard 3.0.
#[tauri::command]
fn export_contacts(path: String, destination: String) -> Result<ExportReport> {
    contacts::export_vcf(Path::new(&path), Path::new(&destination))
}

/// Writes a clean, deduplicated iCalendar 2.0.
#[tauri::command]
fn export_calendar(path: String, destination: String) -> Result<ExportReport> {
    calendar::export_ics(Path::new(&path), Path::new(&destination))
}

/// Space arithmetic, to choose the mode before starting.
///
/// On a large library the question is not whether the operation works, but
/// whether it fits: the repaired copy duplicates everything, in-place does not.
#[tauri::command]
async fn estimate_space(source: String, destination: String) -> Result<SpaceEstimate> {
    in_background(move || compute_space(Path::new(&source), Path::new(&destination))).await
}

/// Margin required on top of the bytes to write, as in `require_free_space`.
const MARGINE_DISCO: f64 = 1.10;

/// Computes the space arithmetic and the slices to divide the work into.
///
/// It lives here and not in `app_state` because telling a year folder from an
/// album needs `albums`, and calling that from the shared state would reverse
/// the direction of the dependencies.
fn compute_space(source: &Path, destination: &Path) -> Result<SpaceEstimate> {
    app_state::require_existing(source)?;

    let mut source_bytes = 0u64;
    let mut largest = 0u64;
    for entry in WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
    {
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        source_bytes += size;
        largest = largest.max(size);
    }

    let mut probe = destination;
    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent,
            None => break,
        }
    }
    let available_bytes = fs4::available_space(probe).unwrap_or(0);
    let needed_for_copy = (source_bytes as f64 * MARGINE_DISCO) as u64;

    // First pass: the names of the media sitting in a year folder.
    // They tell us which photos of an album exist only there.
    let mut nelle_annate: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut cartelle: Vec<(PathBuf, String, albums::FolderKind)> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(source) {
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if name.starts_with('.') {
                continue;
            }
            let kind = albums::classify_folder(&name);

            if matches!(kind, albums::FolderKind::Year(_)) {
                for file in WalkDir::new(&dir)
                    .follow_links(false)
                    .into_iter()
                    .flatten()
                    .filter(|e| e.file_type().is_file())
                {
                    if let Some(name) = file.file_name().to_str() {
                        nelle_annate.insert(name.to_string());
                    }
                }
            }
            cartelle.push((dir, name, kind));
        }
    }

    let mut subfolders: Vec<FolderSize> = Vec::new();
    for (dir, name, kind) in cartelle {
        let is_album = kind == albums::FolderKind::Album;
        let mut bytes = 0u64;
        let mut file_count = 0usize;
        let mut unique_here = 0usize;

        for file in WalkDir::new(&dir)
            .follow_links(false)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
        {
            file_count += 1;
            bytes += file.metadata().map(|m| m.len()).unwrap_or(0);

            // Only for albums does it make sense to ask whether the photo exists elsewhere.
            if is_album && exif_parser::is_media_file(file.path()) {
                let assente = file
                    .file_name()
                    .to_str()
                    .is_none_or(|name| !nelle_annate.contains(name));
                if assente {
                    unique_here += 1;
                }
            }
        }

        subfolders.push(FolderSize {
            name,
            path: dir,
            bytes,
            file_count,
            fits: available_bytes >= (bytes as f64 * MARGINE_DISCO) as u64,
            is_year: matches!(kind, albums::FolderKind::Year(_)),
            is_album,
            unique_here,
        });
    }
    subfolders.sort_by_key(|f| std::cmp::Reverse(f.bytes));

    Ok(SpaceEstimate {
        source_bytes,
        available_bytes,
        needed_for_copy,
        copy_fits: available_bytes >= needed_for_copy,
        // In-place works on one file at a time, per thread.
        needed_in_place: largest.saturating_mul(4).max(64 * 1024 * 1024),
        subfolders,
    })
}

/// Identifying data of the application, for the guide.
#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo::default()
}

/// Path of the preferences file, in the system configuration folder.
fn preferences_path(app: &AppHandle) -> Result<PathBuf> {
    use tauri::Manager;

    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| TakeoutError::ConfigDirUnavailable(e.to_string()))?;
    Ok(dir.join("preferences.json"))
}

/// Reads the preferences. A missing file means default values.
#[tauri::command]
fn read_preferences(app: AppHandle) -> Result<Preferences> {
    let path = preferences_path(&app)?;
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Ok(Preferences::default());
    };
    // A corrupted file must not prevent startup: we fall back to the defaults.
    Ok(serde_json::from_str(&content).unwrap_or_default())
}

/// Saves the preferences.
#[tauri::command]
fn write_preferences(app: AppHandle, preferences: Preferences) -> Result<()> {
    let path = preferences_path(&app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TakeoutError::io(parent, e))?;
    }
    let json = serde_json::to_string_pretty(&preferences)
        .map_err(|e| TakeoutError::Metadata(e.to_string()))?;
    std::fs::write(&path, json).map_err(|e| TakeoutError::io(&path, e))?;
    trace_dev!("preferences saved to {}", path.display());
    Ok(())
}

/// Writes text to a path the user chose in the save dialog.
///
/// The same trust model as the other exports: the path never comes from the
/// frontend's own idea of where to write, it comes from a system dialog the
/// user opened. Kept generic because a problem report is plain text and does
/// not deserve a format of its own.
#[tauri::command]
fn save_text_file(path: String, content: String) -> Result<()> {
    let path = PathBuf::from(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TakeoutError::io(parent, e))?;
    }
    std::fs::write(&path, content).map_err(|e| TakeoutError::io(&path, e))?;
    Ok(())
}

/// Address a problem report is sent to.
///
/// Compiled in and never composed from anything the user or a file supplies:
/// the one argument that reaches the system is a `mailto:` built around this
/// constant, so there is no address an archive could redirect a report to.
const SUPPORT_EMAIL: &str = "support@skapacraft.com";

/// Percent-encodes a string for use inside a `mailto:` URL.
///
/// Written by hand rather than pulled from a crate: it is fifteen lines, and a
/// project that bans network crates by name should not add a dependency to
/// escape two fields.
///
/// Everything outside the unreserved set of RFC 3986 is encoded, which covers
/// the newlines that would otherwise let a subject inject extra mail headers.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Opens the system mail client on a pre-filled problem report.
///
/// This hands a `mailto:` to the operating system, which is not a network
/// connection made by this process: nothing is sent until the user presses send
/// in their own mail client, and they see the whole text first. That
/// distinction is the reason it does not break the promise in section 1 of
/// PRIVACY_AUDIT.md, and it is recorded in section 7b alongside the file
/// manager button.
///
/// `tauri-plugin-opener` stays banned in `deny.toml`: it opens arbitrary URLs
/// in a browser, while this builds one address that cannot be influenced from
/// outside.
#[tauri::command]
fn compose_support_email(subject: String, body: String) -> Result<()> {
    let url = format!(
        "mailto:{}?subject={}&body={}",
        SUPPORT_EMAIL,
        percent_encode(&subject),
        percent_encode(&body)
    );

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg(&url);
        c
    };

    // `rundll32 url.dll,FileProtocolHandler` is the documented way to hand a
    // URL to Windows. `start` would need a shell, and a shell is exactly what
    // this project does not want between itself and the system.
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler").arg(&url);
        c
    };

    #[cfg(target_os = "linux")]
    let mut command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&url);
        c
    };

    command
        .spawn()
        .map_err(|e| TakeoutError::Task(format!("mail client did not start: {e}")))?;
    Ok(())
}

/// The environment a problem report should mention.
///
/// Deliberately small: the operating system, the architecture and the version.
/// No machine name, no user name, no locale, nothing that identifies a person
/// rather than a configuration.
#[tauri::command]
fn report_environment() -> String {
    format!(
        "{} {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        env!("CARGO_PKG_VERSION")
    )
}

/// Replaces the user's home directory with `~` in a diagnostic line.
///
/// Error messages carry the path that failed, which is genuinely useful and
/// also contains the account name. Redacting the prefix keeps the shape of the
/// path, which is what helps, and drops the part that identifies whoever sent
/// the report.
#[tauri::command]
fn redact_home(app: AppHandle, lines: Vec<String>) -> Vec<String> {
    use tauri::Manager;

    let home = app
        .path()
        .home_dir()
        .ok()
        .map(|h| h.display().to_string())
        .unwrap_or_default();

    lines
        .into_iter()
        .map(|line| {
            if home.is_empty() {
                line
            } else {
                line.replace(&home, "~")
            }
        })
        .collect()
}

/// Reveals a path in the system file manager.
///
/// It does not use `tauri-plugin-opener`, which stays banned in `deny.toml`
/// because it can also open URLs in the browser. Here the program invoked is
/// fixed, the only argument is a path that must already exist, and it does not
/// go through a shell: there is no command string the user could influence.
///
/// On Linux it opens the folder and not the file: `xdg-open` on a file would
/// open it with the default application, which is a different thing from
/// revealing it.
#[tauri::command]
fn reveal_in_file_manager(path: String) -> Result<()> {
    let path = Path::new(&path);
    app_state::require_existing(path)?;
    let target = path.canonicalize().map_err(|e| TakeoutError::io(path, e))?;

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(&target);
        c
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{}", target.display()));
        c
    };

    #[cfg(target_os = "linux")]
    let mut command = {
        let folder = if target.is_dir() {
            target.clone()
        } else {
            target.parent().unwrap_or(&target).to_path_buf()
        };
        let mut c = std::process::Command::new("xdg-open");
        c.arg(folder);
        c
    };

    command
        .spawn()
        .map_err(|e| TakeoutError::Task(format!("file manager did not start: {e}")))?;
    Ok(())
}

/// Declaration of the privacy profile, shown in the UI.
#[tauri::command]
fn privacy_report() -> PrivacyReport {
    PrivacyReport::default()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // The only plugin registered: the system file picker. No URL opener,
        // no updater, no channel to the outside.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .setup(|app| {
            let menu = build_menu(app.handle())?;
            app.set_menu(menu)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            // The menu knows nothing of the UI state: it merely asks, and the
            // frontend decides what to show.
            if event.id() == MENU_HELP_ID {
                let _ = app.emit(SHOW_HELP_EVENT, ());
            } else if event.id() == MENU_REPORT_ID {
                let _ = app.emit(SHOW_REPORT_EVENT, ());
            } else if event.id() == MENU_VERSION_ID {
                let _ = app.emit(SHOW_VERSION_EVENT, ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_source,
            current_source,
            close_source,
            inspect_archive,
            list_archive_entries,
            extract_archive,
            discover_archive_series,
            extract_takeout,
            scan_photos,
            repair_photos,
            scan_contacts,
            scan_drive,
            plan_drive_clean,
            clean_drive,
            sweep_sidecars,
            restore_quarantine,
            scan_calendar,
            scan_albums,
            export_album_manifest,
            export_contacts,
            export_calendar,
            privacy_report,
            app_info,
            estimate_space,
            read_preferences,
            write_preferences,
            reveal_in_file_manager,
            compose_support_email,
            report_environment,
            redact_home,
            save_text_file,
        ])
        .run(tauri::generate_context!())
        .expect("errore durante l'avvio dell'applicazione Tauri");
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app_state::testing::{write_bytes, write_file as write, TempDir, MINIMAL_JPEG};

    /// Builds a synthetic Takeout with the three analysable sections.
    fn build_fixture(root: &Path) {
        let takeout = root.join("Takeout");

        // Google Photos: a real JPEG with the sidecar carrying date and position.
        let photos = takeout.join("Google Foto");
        write_bytes(&photos.join("IMG_0001.JPG"), MINIMAL_JPEG);
        write(
            &photos.join("IMG_0001.JPG.json"),
            r#"{
              "title": "IMG_0001.JPG",
              "photoTakenTime": { "timestamp": "1577880000" },
              "geoData": { "latitude": 45.4642, "longitude": 9.19, "altitude": 120.0 }
            }"#,
        );

        // Contacts: two cards, one duplicated by email.
        write(
            &takeout.join("Contatti").join("contacts.vcf"),
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Mario Rossi\r\nEMAIL:mario@example.com\r\nEND:VCARD\r\n\
             BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Mario Rossi\r\nEMAIL:MARIO@example.com\r\nTEL:+39 320 1234567\r\nEND:VCARD\r\n\
             BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Giulia Bianchi\r\nEMAIL:giulia@example.com\r\nEND:VCARD\r\n",
        );

        // Drive: a real document, a placeholder, two identical copies.
        let drive = takeout.join("Drive");
        write(&drive.join("relazione.docx"), "contenuto documento");
        write(
            &drive.join("appunti.gdoc"),
            r#"{"url": "https://docs.google.com/open?id=abc123", "doc_id": "abc123"}"#,
        );
        write(&drive.join("a").join("copy.txt"), "stesso contenuto");
        write(&drive.join("b").join("copy.txt"), "stesso contenuto");
    }

    #[test]
    fn recognises_the_sections_of_a_synthetic_takeout() {
        let temp = TempDir::new("sezioni");
        build_fixture(temp.path());

        // The root is passed as the containing folder: it has to descend into `Takeout/`.
        let summary = analyze_folder(temp.path()).expect("folder analysis");

        assert_eq!(summary.kind, SourceKind::Folder);
        assert!(summary.root.ends_with("Takeout"));
        assert_eq!(summary.sections.len(), 3);
        assert!(summary.warnings.is_empty());
        // 2 in Google Photos (media + sidecar), 1 in Contacts, 4 in Drive.
        assert_eq!(summary.file_count, 7);

        let sections: Vec<TakeoutSection> = summary.sections.iter().map(|s| s.section).collect();
        assert!(sections.contains(&TakeoutSection::GooglePhotos));
        assert!(sections.contains(&TakeoutSection::Contacts));
        assert!(sections.contains(&TakeoutSection::Drive));
    }

    #[test]
    fn reconciles_photo_dates_from_the_sidecar() {
        let temp = TempDir::new("photos");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");

        let report = exif_parser::scan_directory(&photos, SAMPLE_SIZE).expect("scan photos");
        assert_eq!(report.media_count, 1);
        assert_eq!(report.with_sidecar, 1);
        assert_eq!(report.with_exif_date, 0, "the file has no EXIF");
        assert_eq!(report.without_exif, 1);
        assert_eq!(
            report.needs_repair, 1,
            "the date exists only in the sidecar"
        );
        assert_eq!(report.with_geo, 1);

        let media = &report.sample[0];
        assert_eq!(media.taken_at_source, exif_parser::MetadataSource::Sidecar);
        assert_eq!(
            media.resolved_taken_at.expect("data risolta").timestamp(),
            1_577_880_000
        );

        // The dry run counts the candidates without touching the disk.
        let originale_prima = std::fs::read(photos.join("IMG_0001.JPG")).expect("lettura");
        let dry = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions::default(),
            &app_state::no_progress,
        )
        .expect("simulazione");
        assert_eq!(dry.candidates, 1);
        assert_eq!(dry.exif_written, 0);
        assert_eq!(dry.file_times_written, 0);
        assert_eq!(
            std::fs::read(photos.join("IMG_0001.JPG")).unwrap(),
            originale_prima,
            "the dry run must not touch the bytes"
        );
    }

    #[test]
    fn copy_mode_leaves_the_originals_untouched() {
        let temp = TempDir::new("copy");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");
        let output = temp.path().join("riparate");

        let originale_prima = std::fs::read(photos.join("IMG_0001.JPG")).expect("lettura");
        let mtime_prima = std::fs::metadata(photos.join("IMG_0001.JPG"))
            .and_then(|m| m.modified())
            .expect("mtime");

        let report = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                output_root: Some(output.clone()),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("copy riparata");

        assert_eq!(report.candidates, 1);
        assert!(report.failures.is_empty(), "{:?}", report.failures);

        // The original has to be identical, byte for byte and by date.
        assert_eq!(
            std::fs::read(photos.join("IMG_0001.JPG")).unwrap(),
            originale_prima
        );
        assert_eq!(
            std::fs::metadata(photos.join("IMG_0001.JPG"))
                .and_then(|m| m.modified())
                .unwrap(),
            mtime_prima
        );

        // The copy exists and carries the capture date.
        let copy = output.join("IMG_0001.JPG");
        assert!(copy.is_file(), "the copy has to be produced");
        let seconds = std::fs::metadata(&copy)
            .and_then(|m| m.modified())
            .expect("mtime copy")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("epoch")
            .as_secs();
        assert_eq!(seconds, 1_577_880_000);
        assert_eq!(report.file_times_written, 1);
        assert_eq!(report.exif_written, 1);

        // The acid test: the tags written have to be readable back, and the round
        // trip through degrees/minutes/seconds has to preserve the coordinates.
        let riletto = exif_parser::read_exif(&copy).expect("rilettura EXIF");
        assert_eq!(
            riletto.taken_at.expect("data scritta").timestamp(),
            1_577_880_000,
            "the capture date has to be inside the file now"
        );

        let geo = riletto.geo.expect("coordinate scritte");
        assert!(
            (geo.latitude - 45.4642).abs() < 1e-4,
            "latitudine riletta: {}",
            geo.latitude
        );
        assert!(
            (geo.longitude - 9.19).abs() < 1e-4,
            "longitudine riletta: {}",
            geo.longitude
        );

        // The repaired file stays a valid JPEG, not a corrupted container.
        let bytes = std::fs::read(&copy).expect("lettura copy");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "firma JPEG intatta");

        // The decisive test on the time zone: the round trip above would come out
        // right even writing UTC, so the string inside the file has to be looked at.
        // The fixture coordinates are in Milan and the date is in January: on the
        // spot the clock read 13:00, not the 12:00 of UTC.
        let grezzo = String::from_utf8_lossy(&bytes);
        assert!(
            grezzo.contains("2020:01:01 13:00:00"),
            "DateTimeOriginal has to carry the local time of the place"
        );
        assert!(
            grezzo.contains("+01:00"),
            "the offset has to be declared beside the date"
        );
    }

    #[test]
    fn the_copy_includes_formats_without_exif_too() {
        let temp = TempDir::new("copy-video");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");
        let output = temp.path().join("riparate");

        // A video with its sidecar: we cannot write its EXIF, but the output tree
        // has to stay complete, otherwise the user ends up with a copy holding only
        // half their memories.
        write(&photos.join("VID_0001.MP4"), "contenuto video finto");
        write(
            &photos.join("VID_0001.MP4.json"),
            r#"{"photoTakenTime": { "timestamp": "1577880000" }}"#,
        );

        let report = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                output_root: Some(output.clone()),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("copy riparata");

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.skipped_unsupported, 1, "the video has no EXIF");
        assert!(
            output.join("VID_0001.MP4").is_file(),
            "the video still has to be copied into the output tree"
        );
        assert!(output.join("IMG_0001.JPG").is_file());

        // With no EXIF written the date lives only in the mtime, which is fragile:
        // the sidecar has to follow the file, otherwise the only durable source
        // stays behind in the source folder.
        assert_eq!(report.sidecars_copied, 1);
        assert!(
            output.join("VID_0001.MP4.json").is_file(),
            "the video sidecar has to be kept beside the copy"
        );
        // For the JPEG the EXIF was written inside the file: the sidecar would be
        // a duplication and is not carried over.
        assert!(!output.join("IMG_0001.JPG.json").exists());

        // Even without EXIF, the file date has to be aligned.
        let seconds = std::fs::metadata(output.join("VID_0001.MP4"))
            .and_then(|m| m.modified())
            .expect("mtime video")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("epoch")
            .as_secs();
        assert_eq!(seconds, 1_577_880_000);
    }

    /// The mode that rewrites the originals is the only irreversible one, so it
    /// is the one deserving the harshest test: it has to write the tags and align
    /// the date **without** altering a single byte of the image.
    #[test]
    fn in_place_mode_rewrites_without_touching_the_pixels() {
        let temp = TempDir::new("in-place");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");
        let originale = photos.join("IMG_0001.JPG");

        /// Extracts the compressed data of the main image, stepping over the header
        /// segments. The thumbnail inside the EXIF has an SOS marker of its own, so
        /// looking for the first `FFDA` would give the wrong result.
        /// sbagliato.
        fn scan_data(path: &Path) -> Vec<u8> {
            let bytes = std::fs::read(path).expect("lettura JPEG");
            let mut i = 2; // dopo SOI
            while i < bytes.len() - 1 && bytes[i] == 0xFF {
                let marker = bytes[i + 1];
                // TEM, RST0-7 and SOI have no length field.
                if marker == 0x01 || (0xD0..=0xD8).contains(&marker) {
                    i += 2;
                    continue;
                }
                let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
                if marker == 0xDA {
                    return bytes[i + 2 + len..].to_vec();
                }
                i += 2 + len;
            }
            panic!("SOS marker not found");
        }

        let pixel_prima = scan_data(&originale);
        assert!(!pixel_prima.is_empty());

        let report = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::InPlace,
                output_root: None,
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("riscrittura in place");

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.exif_written, 1);
        assert_eq!(report.file_times_written, 1);
        // In this mode nothing is copied: the sidecar stays where it is.
        assert_eq!(report.sidecars_copied, 0);

        // The file was rewritten in place, not duplicated elsewhere.
        assert!(originale.is_file());
        assert!(!photos.join(".oth-tmp-IMG_0001.JPG").exists());

        // The image data has to be identical byte for byte.
        assert_eq!(
            scan_data(&originale),
            pixel_prima,
            "the in-place rewrite altered the pixels"
        );

        // The tags really are inside the original now.
        let riletto = exif_parser::read_exif(&originale).expect("rilettura");
        assert_eq!(
            riletto.taken_at.expect("data scritta").timestamp(),
            1_577_880_000
        );
        let geo = riletto.geo.expect("coordinate scritte");
        assert!((geo.latitude - 45.4642).abs() < 1e-4);

        let seconds = std::fs::metadata(&originale)
            .and_then(|m| m.modified())
            .expect("mtime")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("epoch")
            .as_secs();
        assert_eq!(seconds, 1_577_880_000);
    }

    #[test]
    fn lays_the_output_out_chronologically() {
        let temp = TempDir::new("layout");
        let photos = temp.path().join("photos");
        let output = temp.path().join("cronologico");

        // Two photos sharing a name in different folders, as happens when the same
        // image sits in an album and in a year folder, plus one with no derivable
        // date at all.
        write_bytes(&photos.join("a").join("IMG_1.JPG"), MINIMAL_JPEG);
        write(
            &photos.join("a").join("IMG_1.JPG.json"),
            r#"{"photoTakenTime": { "timestamp": "1577880000" }}"#,
        );
        write_bytes(&photos.join("b").join("IMG_1.JPG"), MINIMAL_JPEG);
        write(
            &photos.join("b").join("IMG_1.JPG.json"),
            r#"{"photoTakenTime": { "timestamp": "1583064000" }}"#,
        );
        write_bytes(&photos.join("senza.JPG"), MINIMAL_JPEG);

        let report = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                layout: exif_parser::OutputLayout::ByYearMonth,
                output_root: Some(output.clone()),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("output cronologica");
        assert!(report.failures.is_empty(), "{:?}", report.failures);

        // 2020-01-01 and 2020-03-01 end up in different months, so there is no
        // collision despite the identical name.
        assert!(output.join("2020/01/IMG_1.JPG").is_file());
        assert!(output.join("2020/03/IMG_1.JPG").is_file());

        // Anything without a date is not filed under an invented month.
        assert!(output.join("no-date/senza.JPG").is_file());
    }

    #[test]
    fn does_not_overwrite_equal_names_in_the_same_folder() {
        let temp = TempDir::new("collisioni-layout");
        let photos = temp.path().join("photos");
        let output = temp.path().join("piatto");

        // Same name, same date, different folders: in the flat layout they would
        // end up on the same path.
        for sottocartella in ["a", "b", "c"] {
            write_bytes(&photos.join(sottocartella).join("IMG_1.JPG"), MINIMAL_JPEG);
            write(
                &photos.join(sottocartella).join("IMG_1.JPG.json"),
                r#"{"photoTakenTime": { "timestamp": "1577880000" }}"#,
            );
        }

        let report = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                layout: exif_parser::OutputLayout::Flat,
                output_root: Some(output.clone()),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("output piatta");
        assert!(report.failures.is_empty(), "{:?}", report.failures);

        // All three have to survive, with an incrementing counter.
        let produced = std::fs::read_dir(&output)
            .expect("lettura output")
            .flatten()
            .count();
        assert_eq!(produced, 3, "no photo may be overwritten");
        assert!(output.join("IMG_1.JPG").is_file());
        assert!(output.join("IMG_1 (2).JPG").is_file());
        assert!(output.join("IMG_1 (3).JPG").is_file());
    }

    /// On a library of tens of gigabytes the repaired copy needs as many again:
    /// if the disk fills halfway, the output tree looks complete and is not.
    /// Better to refuse before starting.
    /// Working in slices, the temptation is to repair only the year folders,
    /// because albums are almost all copies. That "almost" is the point: a photo
    /// sitting only in an album would be left behind, and whoever looks at the
    /// result would not notice.
    #[test]
    fn tells_years_from_albums_and_counts_unique_photos() {
        let temp = TempDir::new("tranche");
        let photos = temp.path().join("Google Foto");

        write_bytes(&photos.join("Foto da 2020").join("IMG_1.JPG"), MINIMAL_JPEG);
        write_bytes(&photos.join("Foto da 2020").join("IMG_2.JPG"), MINIMAL_JPEG);
        // An album with one copy and one photo that exists nowhere else.
        write_bytes(&photos.join("Vacanze").join("IMG_1.JPG"), MINIMAL_JPEG);
        write_bytes(&photos.join("Vacanze").join("SOLO_QUI.JPG"), MINIMAL_JPEG);
        // An album made only of copies: skipping it costs nothing.
        write_bytes(&photos.join("Compleanno").join("IMG_2.JPG"), MINIMAL_JPEG);

        let stima = compute_space(&photos, temp.path()).expect("stima");

        let annata = stima
            .subfolders
            .iter()
            .find(|f| f.name == "Foto da 2020")
            .expect("annata");
        assert!(annata.is_year && !annata.is_album);
        assert_eq!(
            annata.unique_here, 0,
            "for year folders the question does not arise"
        );

        let vacanze = stima
            .subfolders
            .iter()
            .find(|f| f.name == "Vacanze")
            .expect("album");
        assert!(vacanze.is_album && !vacanze.is_year);
        assert_eq!(
            vacanze.unique_here, 1,
            "SOLO_QUI exists in no year folder: skipping this album would lose it"
        );

        let compleanno = stima
            .subfolders
            .iter()
            .find(|f| f.name == "Compleanno")
            .expect("album");
        assert_eq!(
            compleanno.unique_here, 0,
            "copies only: it can be skipped without losing anything"
        );
    }

    #[test]
    fn refuses_when_the_destination_lacks_space() {
        let temp = TempDir::new("spazio");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");

        // An absurd space requirement cannot be available anywhere, so the check
        // has to trigger.
        let outcome = app_state::require_free_space(temp.path(), u64::MAX / 2);
        assert!(outcome.is_err(), "it has to be refused before writing");
        let message = outcome.unwrap_err().to_string();
        assert!(
            message.contains("not enough space"),
            "the message has to say what is missing: {message}"
        );

        // With a plausible request it passes instead, and the repair proceeds.
        app_state::require_free_space(temp.path(), 1024).expect("mille byte ci stanno");

        let report = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                output_root: Some(temp.path().join("output")),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("repair");
        assert!(report.failures.is_empty());
    }

    #[test]
    fn refuses_a_destination_inside_the_source() {
        let temp = TempDir::new("ricorsione");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");

        let outcome = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                output_root: Some(photos.join("output")),
                ..Default::default()
            },
            &app_state::no_progress,
        );

        assert!(outcome.is_err(), "a nested destination has to be refused");
    }

    #[test]
    fn deduplicates_contacts() {
        let temp = TempDir::new("contacts");
        build_fixture(temp.path());

        let report =
            contacts::scan_directory(&temp.path().join("Takeout").join("Contatti"), SAMPLE_SIZE)
                .expect("scan contacts");

        assert_eq!(report.total, 3);
        assert_eq!(
            report.duplicates, 1,
            "the two cards for Mario are the same person"
        );
        assert_eq!(report.unique, 2);
        assert_eq!(report.with_email, 3);

        // The merge must not lose the phone present only in the duplicate.
        let mario = report
            .sample
            .iter()
            .find(|c| c.display_name.as_deref() == Some("Mario Rossi"))
            .expect("contatto fuso");
        assert_eq!(mario.phones.len(), 1);
    }

    #[test]
    fn flags_drive_placeholders_and_duplicates() {
        let temp = TempDir::new("drive");
        build_fixture(temp.path());

        let report = drive::scan_directory(&temp.path().join("Takeout").join("Drive"), MAX_ITEMS)
            .expect("scan drive");

        assert_eq!(report.file_count, 4);
        assert_eq!(report.placeholder_count, 1);
        assert_eq!(
            report.placeholders[0].target_url.as_deref(),
            Some("https://docs.google.com/open?id=abc123")
        );
        assert_eq!(report.duplicate_groups.len(), 1);
        assert_eq!(report.duplicate_groups[0].paths.len(), 2);
        assert!(
            !report.warnings.is_empty(),
            "the placeholder has to be reported"
        );
    }

    /// Measures behaviour on a library of realistic size.
    ///
    /// Excluded from CI because it generates tens of thousands of files. Run it by
    /// hand, choosing how many photos to produce if you like:
    ///
    /// ```bash
    /// PHOTOS=50000 cargo test --release measures_a_large_library -- --ignored --nocapture
    /// ```
    ///
    /// It has to run in release, and not out of fussiness: in debug the Rust code
    /// is more than an order of magnitude slower, so the timings would say nothing
    /// useful, and the development diagnostics would print one line per file,
    /// drowning the result.
    ///
    /// It is worth spelling out why a hundred gigabyte export is unnecessary. What
    /// puts this code under strain is the **number of files**, not the number of
    /// bytes: a hundred gigabytes of video is two hundred files, which is nothing.
    /// With an 889 byte JPEG you generate a hundred thousand of them in a hundred
    /// and fifty megabytes, obtaining a harsher test than a real library of the
    /// same size.
    ///
    /// The measurement that matters most is not the time but the size of the
    /// serialised reports: they are what crosses the IPC channel towards the
    /// interface on every scan, and an uncapped list there becomes megabytes of JSON.
    #[test]
    #[ignore = "genera decine di migliaia di file: si lancia a mano"]
    fn measures_a_large_library() {
        use std::time::Instant;

        let total_photos: usize = std::env::var("PHOTOS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20_000);

        let temp = TempDir::new("scala");
        let root = temp.path().join("Takeout").join("Google Foto");

        // A structure resembling a real export: year folders, albums repeating part
        // of the photos, edited versions, and a share with no derivable date.
        //
        let years = [2019, 2020, 2021, 2022, 2023];
        let album = [
            "Vacanze in Sicilia",
            "Compleanno di Anna",
            "Montagna 2021",
            "Matrimonio",
        ];

        let started = Instant::now();
        let mut names: Vec<(String, usize)> = Vec::with_capacity(total_photos);

        /// Produces a valid JPEG different from every other one, keeping the same
        /// size.
        ///
        /// The bytes trailing after the end marker are ignored by decoders, so the
        /// file stays readable. It exists to keep the fixture from being all
        /// identical: in that case the hash would merge them into one enormous group
        /// and the measurement would say nothing about the real case. With equal size
        /// and different content you get instead the most demanding scenario for
        /// deduplication, which has to read every file to discover they are all
        /// distinct.
        fn unique_jpeg(index: usize) -> Vec<u8> {
            let mut bytes = MINIMAL_JPEG.to_vec();
            bytes.extend_from_slice(&(index as u64).to_le_bytes());
            bytes
        }

        for index in 0..total_photos {
            let anno = years[index % years.len()];
            let mese = (index % 12) + 1;
            let giorno = (index % 28) + 1;
            let name = format!("IMG_{anno}{mese:02}{giorno:02}_{:06}.JPG", index % 240_000);
            let folder = root.join(format!("Foto da {anno}"));

            write_bytes(&folder.join(&name), &unique_jpeg(index));

            // One photo in five is left without a sidecar: it will have to make do
            // with the date derived from the name.
            if index % 5 != 0 {
                let istante = 1_577_880_000 + (index as i64 * 37);
                // One in three has coordinates, so it goes through the time zone
                // conversion, which is the most expensive path.
                let geo = if index % 3 == 0 {
                    r#", "geoData": {"latitude": 45.4642, "longitude": 9.19, "altitude": 120.0}"#
                } else {
                    ""
                };
                write(
                    &folder.join(format!("{name}.supplemental-metadata.json")),
                    &format!(r#"{{"photoTakenTime": {{"timestamp": "{istante}"}}{geo}}}"#),
                );
            }

            // One photo in twenty has an edited version beside it.
            if index % 20 == 0 {
                let modificata = name.replace(".JPG", "-modificato.JPG");
                // An edited version has different pixels: it is not a duplicate.
                write_bytes(
                    &folder.join(&modificata),
                    &unique_jpeg(index + total_photos),
                );
            }

            names.push((name, index));
        }

        // A tenth of the photos also appear in an album: the case that makes the
        // manifest necessary.
        // These are the genuine duplicates: an identical copy of the photo already
        // sitting in the year folder.
        for (name, index) in names.iter().filter(|(_, i)| i % 10 == 0) {
            let scelto = album[index % album.len()];
            write_bytes(&root.join(scelto).join(name), &unique_jpeg(*index));
        }

        let file_totali = WalkDir::new(&root)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .count();
        let total_bytes: u64 = WalkDir::new(&root)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();

        println!("\n=== libreria generata ===");
        println!("  photos:        {total_photos}");
        println!("  file totali: {file_totali}");
        println!("  byte:        {:.1} MB", total_bytes as f64 / 1e6);
        println!("  generazione: {:.1} s", started.elapsed().as_secs_f64());

        /// Measures duration and weight of the serialised report, that is how much
        /// really goes through the IPC channel.
        fn measure<T: serde::Serialize>(name: &str, work: impl FnOnce() -> T) {
            let started = Instant::now();
            let outcome = work();
            let elapsed = started.elapsed();
            let json = serde_json::to_string(&outcome).unwrap_or_default();
            println!(
                "  {name:<22} {:>7.2} s   report {:>8.2} MB",
                elapsed.as_secs_f64(),
                json.len() as f64 / 1e6
            );
        }

        println!("\n=== operazioni ===");
        measure("scan photos", || {
            exif_parser::scan_directory(&root, SAMPLE_SIZE).expect("scan")
        });
        measure("index album", || {
            albums::build_index(&root, MAX_ITEMS).expect("index")
        });
        measure("plan di pulizia", || {
            drive::plan_clean(
                &root,
                &drive::CleanOptions::default(),
                MAX_ITEMS,
                &app_state::no_progress,
            )
            .expect("plan")
        });
        println!();
    }

    /// Measures the real-bytes path: hashing, copying and the rewrite threshold.
    ///
    /// It is the other half of the scale test. The one on a hundred thousand photos
    /// verifies the cost per file; this verifies the cost per byte, which is
    /// dominated by the disk but touches two points of ours: `little_exif` loads
    /// the whole file to rewrite into memory, and there are four threads, so the
    /// peak grows with the size of the individual media.
    ///
    /// ```bash
    /// GB=2 cargo test --release measures_large_files -- --ignored --nocapture
    /// ```
    ///
    /// The memory peak is not measured from here: it is read from outside.
    ///
    /// ```bash
    /// /usr/bin/time -l cargo test --release measures_large_files -- --ignored --nocapture
    /// ```
    ///
    /// Mind which number you look at. The `maximum resident set size` on macOS
    /// includes the pages of the files touched, which belong to the kernel cache
    /// and are reclaimed under pressure: it grows with the amount of data read and
    /// written, varies between two identical runs, and says nothing about how much
    /// the program allocates. The figure that counts is `peak memory footprint`,
    /// which measures anonymous memory: on two and a half gigabytes of media it
    /// stays around a hundred megabytes, that is within the ceiling the four
    /// threads and the rewrite threshold ought to guarantee.
    #[test]
    #[ignore = "scrive qualche gigabyte: si lancia a mano"]
    fn measures_large_files() {
        use std::io::Write;
        use std::time::Instant;

        /// Size of media that are large but still rewritable.
        ///
        /// Below the 128 MB threshold, so they really do go through the EXIF
        /// rewrite: that is the case testing memory, because every thread holds a
        /// copy of one.
        const GRANDE: u64 = 64 * 1024 * 1024;
        /// Above the threshold: it has to be skipped, not rewritten.
        const ENORME: u64 = 200 * 1024 * 1024;

        let gigabyte: u64 = std::env::var("GB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2);
        let count = ((gigabyte * 1024 * 1024 * 1024) / GRANDE).max(2) as usize;

        let temp = TempDir::new("byte");
        let root = temp.path().join("Foto da 2024");
        let output = temp.path().join("riparate");

        /// Writes a valid JPEG of the requested size.
        ///
        /// The bytes after the end marker are ignored by decoders, so the file stays
        /// readable. The padding depends on the seed, so two files of the same size
        /// have different content and deduplication has to read them whole to find
        /// out.
        fn write_large(path: &Path, dimensione: u64, seme: u8) {
            std::fs::create_dir_all(path.parent().expect("genitore")).expect("cartelle");
            let file = std::fs::File::create(path).expect("creazione");
            let mut out = std::io::BufWriter::with_capacity(8 << 20, file);
            out.write_all(MINIMAL_JPEG).expect("intestazione");

            let blocco = vec![seme; 8 << 20];
            let mut written = MINIMAL_JPEG.len() as u64;
            while written < dimensione {
                let count = (dimensione - written).min(blocco.len() as u64) as usize;
                out.write_all(&blocco[..count]).expect("riempimento");
                written += count as u64;
            }
            out.flush().expect("flush");
        }

        let started = Instant::now();
        for index in 0..count {
            let name = format!("GRANDE_{index:03}.JPG");
            write_large(&root.join(&name), GRANDE, index as u8);
            // Sidecars differing from one another, as in a real export: the title
            // reports the file name and the instant changes with every photo.
            write(
                &root.join(format!("{name}.supplemental-metadata.json")),
                &format!(
                    r#"{{"title": "{name}", "photoTakenTime": {{"timestamp": "{}"}}, "geoData": {{"latitude": 45.4642, "longitude": 9.19, "altitude": 120.0}}}}"#,
                    1_577_880_000_i64 + index as i64
                ),
            );
        }
        // An identical copy of the first: a genuine duplicate to find by content.
        write_large(&temp.path().join("Album").join("GRANDE_000.JPG"), GRANDE, 0);
        // And one past the threshold, which has to be skipped by the rewrite.
        write_large(&root.join("ENORME.JPG"), ENORME, 200);

        let total_bytes: u64 = WalkDir::new(temp.path())
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        let generazione = started.elapsed().as_secs_f64();

        println!("\n=== libreria generata ===");
        println!("  large media: {count} of {} MB", GRANDE / 1024 / 1024);
        println!("  plus one of:  {} MB", ENORME / 1024 / 1024);
        println!("  total:       {:.2} GB", total_bytes as f64 / 1e9);
        println!(
            "  scrittura:    {generazione:.1} s  ({:.0} MB/s)",
            total_bytes as f64 / 1e6 / generazione
        );

        println!("\n=== operazioni ===");

        let started = Instant::now();
        let plan = drive::plan_clean(
            temp.path(),
            &drive::CleanOptions::default(),
            MAX_ITEMS,
            &app_state::no_progress,
        )
        .expect("plan");
        let elapsed = started.elapsed().as_secs_f64();
        println!(
            "  deduplica     {elapsed:>7.2} s   letti {:.2} GB  ({:.0} MB/s)",
            plan.hashed_bytes as f64 / 1e9,
            plan.hashed_bytes as f64 / 1e6 / elapsed
        );
        assert_eq!(
            plan.duplicate_copies, 1,
            "the identical copy has to be found"
        );

        let started = Instant::now();
        let report = exif_parser::apply_metadata(
            &root,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                output_root: Some(output.clone()),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("repair");
        let elapsed = started.elapsed().as_secs_f64();
        let written: u64 = WalkDir::new(&output)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        println!(
            "  repair   {elapsed:>7.2} s   written {:.2} GB  ({:.0} MB/s)",
            written as f64 / 1e9,
            written as f64 / 1e6 / elapsed
        );

        println!("\n=== outcome repair ===");
        println!("  EXIF written:        {}", report.exif_written);
        println!("  past the threshold:  {}", report.skipped_too_large);
        println!("  errori:              {}", report.failures.len());
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(
            report.exif_written, count,
            "i media sotto soglia si riscrivono"
        );
        assert_eq!(
            report.skipped_too_large, 1,
            "the one past the threshold has to be skipped"
        );
        // Skipping it does not mean losing it: the copy has to be there anyway.
        assert!(
            output.join("ENORME.JPG").is_file(),
            "the file past the threshold has to be copied anyway"
        );
        println!();
    }

    /// Measures contacts and calendar at realistic volumes.
    ///
    /// They have the opposite profile to photos: very few files, but large, and
    /// each is read into memory whole before being parsed. The risk is not the
    /// number of opens, it is the size of the single file and the cost of
    /// deduplication, which compares every card with the ones already seen.
    ///
    /// ```bash
    /// CONTACTS=20000 EVENTS=50000 cargo test --release measures_a_large_address_book -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "genera file di decine di megabyte: si lancia a mano"]
    fn measures_a_large_address_book() {
        use std::time::Instant;

        let contacts: usize = std::env::var("CONTACTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20_000);
        let events: usize = std::env::var("EVENTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50_000);

        let temp = TempDir::new("address_book");
        let root = temp.path().join("Takeout");

        let started = Instant::now();

        // Address book: a single .vcf, the way Google exports it. One card in ten is
        // a duplicate with the same email, the case deduplication has to recognise;
        // one in seven has a long line, forcing the parser to rejoin the line
        // folding.
        let mut vcf = String::with_capacity(contacts * 180);
        for index in 0..contacts {
            // One card in ten repeats the identity of the previous one: the real case
            // of someone who saved the same contact twice.
            let chi = if index % 10 == 9 { index - 1 } else { index };
            vcf.push_str("BEGIN:VCARD\r\nVERSION:3.0\r\n");
            vcf.push_str(&format!("FN:Persona Numero {chi}\r\n"));
            vcf.push_str(&format!("N:Numero;Persona{chi};;;\r\n"));
            vcf.push_str(&format!("EMAIL;TYPE=INTERNET:persona{chi}@example.com\r\n"));
            vcf.push_str(&format!(
                "TEL;TYPE=CELL:+39 320 {:07}\r\n",
                chi % 10_000_000
            ));
            if index % 7 == 0 {
                // A line split according to the folding rule.
                vcf.push_str("NOTE:Appunto lungo che continua\r\n  sulla riga successiva\r\n");
            }
            vcf.push_str("END:VCARD\r\n");
        }
        write(&root.join("Contatti").join("Tutti i contatti.vcf"), &vcf);

        // Calendar: five files, like five calendars in the account. One event in
        // eight is recurring, one in twenty is all-day, and each carries
        // proprietary Google properties to strip.
        for calendario in 0..5 {
            let count = events / 5;
            let mut ics = String::with_capacity(count * 260);
            ics.push_str("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Google Inc//EN\r\n");
            for index in 0..count {
                let giorno = (index % 28) + 1;
                let mese = (index % 12) + 1;
                let anno = 2018 + (index % 8);
                ics.push_str("BEGIN:VEVENT\r\n");
                ics.push_str(&format!("UID:evento-{calendario}-{index}@google.com\r\n"));
                if index % 20 == 0 {
                    ics.push_str(&format!(
                        "DTSTART;VALUE=DATE:{anno}{mese:02}{giorno:02}\r\n"
                    ));
                } else {
                    ics.push_str(&format!(
                        "DTSTART;TZID=Europe/Rome:{anno}{mese:02}{giorno:02}T090000\r\n"
                    ));
                    ics.push_str(&format!(
                        "DTEND;TZID=Europe/Rome:{anno}{mese:02}{giorno:02}T100000\r\n"
                    ));
                }
                ics.push_str(&format!("SUMMARY:Impegno numero {index}\r\n"));
                ics.push_str("LOCATION:Ufficio\r\n");
                ics.push_str("X-GOOGLE-CONFERENCE:https://meet.google.com/abc-defg-hij\r\n");
                if index % 8 == 0 {
                    ics.push_str("RRULE:FREQ=WEEKLY;COUNT=10\r\n");
                }
                ics.push_str(
                    "BEGIN:VALARM\r\nACTION:DISPLAY\r\nSUMMARY:Promemoria\r\nEND:VALARM\r\n",
                );
                ics.push_str("END:VEVENT\r\n");
            }
            ics.push_str("END:VCALENDAR\r\n");
            write(
                &root
                    .join("Calendario")
                    .join(format!("calendario-{calendario}.ics")),
                &ics,
            );
        }

        let byte: u64 = WalkDir::new(&root)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();

        println!("\n=== generati ===");
        println!("  contacts:  {contacts} in un solo .vcf");
        println!("  events:    {events} in 5 .ics");
        println!("  byte:      {:.1} MB", byte as f64 / 1e6);
        println!("  scrittura: {:.1} s", started.elapsed().as_secs_f64());

        fn measure<T: serde::Serialize>(name: &str, work: impl FnOnce() -> T) -> T {
            let started = Instant::now();
            let outcome = work();
            let json = serde_json::to_string(&outcome).unwrap_or_default();
            println!(
                "  {name:<24} {:>6.2} s   report {:>7.2} MB",
                started.elapsed().as_secs_f64(),
                json.len() as f64 / 1e6
            );
            outcome
        }

        println!("\n=== operazioni ===");
        let address_book = measure("scan contacts", || {
            contacts::scan_directory(&root.join("Contatti"), SAMPLE_SIZE).expect("contacts")
        });
        let calendar_report = measure("scan calendario", || {
            calendar::scan_directory(&root.join("Calendario"), SAMPLE_SIZE).expect("calendario")
        });

        let output = temp.path().join("output");
        measure("export vCard", || {
            contacts::export_vcf(&root.join("Contatti"), &output.join("contacts.vcf"))
                .expect("export contacts")
        });
        measure("export iCalendar", || {
            calendar::export_ics(&root.join("Calendario"), &output.join("calendario.ics"))
                .expect("export calendario")
        });

        println!("\n=== outcome ===");
        println!(
            "  contacts: {} read, {} unique, {} duplicates",
            address_book.total, address_book.unique, address_book.duplicates
        );
        println!(
            "  events:   {} read, {} unique, {} properties removed",
            calendar_report.total, calendar_report.unique, calendar_report.dropped_properties
        );

        // Deduplication has to recognise the repeated cards, not count them at random.
        assert!(
            address_book.duplicates > 0,
            "the duplicates have to be found"
        );
        assert_eq!(address_book.total, contacts);
        assert_eq!(calendar_report.total, events);
        // The alarms must not be mistaken for events.
        assert!(
            calendar_report.sample.iter().all(|e| e
                .summary
                .as_deref()
                .is_some_and(|s| s.starts_with("Impegno"))),
            "the VALARM SUMMARY must not overwrite the event one"
        );
        println!();
    }

    /// Extracts a multi-archive series taken from disk and analyses the result.
    ///
    ///
    /// Unlike the other measurements this one generates nothing: it works on
    /// archives that already exist, so as to exercise the complete path of series
    /// recognition, merging, photo scanning and albums on material that was not
    /// built by the very tests verifying it.
    ///
    /// ```bash
    /// SERIES=~/Downloads/prova-multiarchivio OUTPUT=~/Downloads/extracted \
    ///   cargo test --release extracts_a_real_series -- --ignored --nocapture
    /// ```
    ///
    /// `SERIES` can point at the folder holding the archives or at any one of them:
    /// series recognition starts from a single archive and finds the others by
    /// itself, and that behaviour is precisely what this exercises. Without
    /// `OUTPUT` the extraction ends up in a temporary folder, removed at the end;
    /// giving it instead lets the extracted tree be reused for later runs.
    ///
    ///
    /// The test is excluded from CI because it depends on local files and, on a
    /// real series, writes as much as the export weighs.
    #[test]
    #[ignore = "needs archives on disk: run by hand with SERIES=..."]
    fn extracts_a_real_series() {
        use std::time::Instant;

        let Ok(series_path) = std::env::var("SERIES") else {
            println!("SERIES not set: nothing to extract.");
            return;
        };
        let series_path = PathBuf::from(series_path);

        // Any archive of the series will do: it finds the rest by itself.
        let first = if series_path.is_dir() {
            let mut archivi: Vec<PathBuf> = std::fs::read_dir(&series_path)
                .expect("reading the folder given")
                .filter_map(|v| v.ok().map(|v| v.path()))
                .filter(|p| zip_handler::is_takeout_archive(p))
                .collect();
            archivi.sort();
            archivi
                .into_iter()
                .next()
                .expect("nessun takeout-*.zip nella folder indicata")
        } else {
            series_path.clone()
        };

        let started = Instant::now();
        let found = zip_handler::discover_series(&first).expect("series recognition");
        println!(
            "\nserie riconosciuta partendo da {}",
            first.file_name().unwrap_or_default().to_string_lossy()
        );
        println!(
            "  {} archives, {:.2} GB compressed, missing: {:?}  ({:?})",
            found.archives.len(),
            found.total_compressed_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
            found.missing,
            started.elapsed()
        );
        assert!(
            found.missing.is_empty(),
            "the series on disk turns out to be incomplete"
        );

        // With OUTPUT the tree stays available, otherwise it disappears.
        let chosen = std::env::var("OUTPUT").ok();
        let temporary = chosen.is_none().then(|| TempDir::new("series_path-reale"));
        let destination = match (&chosen, &temporary) {
            (Some(percorso), _) => PathBuf::from(percorso),
            (None, Some(temp)) => temp.path().join("extracted"),
            (None, None) => unreachable!("without OUTPUT the temporary always exists"),
        };

        let started = Instant::now();
        let extracted = zip_handler::extract_series(
            &found.archives,
            &destination,
            &crate::app_state::no_progress,
        )
        .expect("series extraction");
        let elapsed = started.elapsed();
        let gb = extracted.bytes_written as f64 / 1024.0 / 1024.0 / 1024.0;
        println!("estrazione in {elapsed:?}");
        println!(
            "  {} files, {} folders, {:.2} GB, {:.0} MB/s",
            extracted.files_written,
            extracted.dirs_created,
            gb,
            gb * 1024.0 / elapsed.as_secs_f64()
        );
        println!(
            "  discarded for safety: {}, collisions: {}",
            extracted.skipped.len(),
            extracted.collisions.len()
        );
        for voce in extracted.skipped.iter().take(5) {
            println!("    discarded: {voce}");
        }
        for voce in extracted.collisions.iter().take(5) {
            println!("    collision: {voce}");
        }

        // A merged Takeout has to have a single root, not one per archive.
        let root = extracted.destination.join("Takeout");
        assert!(
            root.is_dir(),
            "the Takeout root is missing from the merged tree"
        );

        let started = Instant::now();
        let source = analyze_folder(&extracted.destination).expect("analysis of the merged tree");
        println!("section analysis in {:?}", started.elapsed());
        for sezione in &source.sections {
            println!(
                "  {:<16?} {:>6} file, {:>6.2} GB",
                sezione.section,
                sezione.file_count,
                sezione.total_bytes as f64 / 1024.0 / 1024.0 / 1024.0
            );
        }

        let photos = root.join("Google Foto");
        if photos.is_dir() {
            let started = Instant::now();
            let index = albums::build_index(&photos, 200).expect("album index");
            println!("albums in {:?}", started.elapsed());
            println!(
                "  {} albums, {} year folders, {} edited pairs",
                index.albums.len(),
                index.year_folders.len(),
                index.edited_pairs.len()
            );
            assert!(
                !index.albums.is_empty() && !index.year_folders.is_empty(),
                "years and albums both have to be identified"
            );

            let started = Instant::now();
            let scan = exif_parser::scan_directory(&photos, SAMPLE_SIZE).expect("scan");
            println!("photo scan in {:?}", started.elapsed());
            println!(
                "  {} media, {:.2} GB, {} with sidecar, {} with coordinates",
                scan.media_count,
                scan.total_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
                scan.with_sidecar,
                scan.with_geo
            );
            println!(
                "  {} to repair, {} without EXIF, {} dated from the name, {} unreadable",
                scan.needs_repair,
                scan.without_exif,
                scan.date_from_filename,
                scan.unreadable_count
            );

            // The sidecars Google generates exist for almost every media file: if few
            // turned up here, recognition of the name truncated to 46 characters would
            // have stopped working.
            assert!(
                scan.with_sidecar * 10 > scan.media_count * 8,
                "too many media without a sidecar: {} out of {}",
                scan.with_sidecar,
                scan.media_count
            );
        }
        println!();
    }

    /// Repairs a real folder and then sets aside the applied sidecars.
    ///
    /// It works on a copy of the folder given, so the original stays untouched and
    /// the measurement can be repeated. It exists to see the whole chain at work,
    /// rewriting included, on files that were not built by the test itself:
    ///
    ///
    /// ```bash
    /// FOLDER="~/Downloads/prova-estratta/Takeout/Google Foto/Foto da 2019" \
    ///   cargo test --release repairs_then_sets_the_sidecars_aside -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a folder on disk: run by hand with FOLDER=..."]
    fn repairs_then_sets_the_sidecars_aside() {
        use std::time::Instant;

        let Ok(source) = std::env::var("FOLDER") else {
            println!("FOLDER not set: nothing to repair.");
            return;
        };
        let source = PathBuf::from(source);

        let temp = TempDir::new("ripara-e-sposta");
        let work = temp.path().join("photos");
        let started = Instant::now();
        let copied = copy_tree(&source, &work);
        println!("\ncopia di work: {copied} file in {:?}", started.elapsed());

        let started = Instant::now();
        let repair = exif_parser::apply_metadata(
            &work,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::InPlace,
                ..Default::default()
            },
            &crate::app_state::no_progress,
        )
        .expect("repair");
        println!("repair in {:?}", started.elapsed());
        println!(
            "  {} candidates, {} EXIF written, {} dates aligned, {} errors",
            repair.candidates,
            repair.exif_written,
            repair.file_times_written,
            repair.failures.len()
        );

        let set_aside = temp.path().join("sidecar");
        let started = Instant::now();
        let sweep =
            drive::sweep_applied_sidecars(&work, &set_aside, 20, &crate::app_state::no_progress)
                .expect("setting the sidecars aside");
        println!("set aside in {:?}", started.elapsed());
        println!(
            "  {} moved ({:.1} kB), {} kept",
            sweep.moved,
            sweep.bytes_moved as f64 / 1024.0,
            sweep.kept
        );
        for motivo in &sweep.kept_reasons {
            println!("    {:>5} per {:?}", motivo.count, motivo.reason);
        }
        assert!(sweep.failures.is_empty(), "{:?}", sweep.failures);

        // What was repaired must not be left behind, and what was not repaired must
        // not be touched.
        assert!(
            sweep.moved > 0,
            "a successful repair has to free some sidecars"
        );

        let started = Instant::now();
        let restored = drive::restore_quarantine(&sweep.manifest.clone().expect("ledger written"))
            .expect("restored");
        println!("restore in {:?}", started.elapsed());
        assert_eq!(
            restored.restored, sweep.moved,
            "the restore has to put back everything that was moved"
        );
        assert!(restored.failures.is_empty(), "{:?}", restored.failures);
        println!();
    }

    /// Copies a folder with everything inside it, returning the files written.
    fn copy_tree(source: &Path, destination: &Path) -> usize {
        let mut written = 0;
        for entry in walkdir::WalkDir::new(source)
            .follow_links(false)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
        {
            let relative = entry.path().strip_prefix(source).expect("relative path");
            let target = destination.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).expect("destination folder");
            }
            std::fs::copy(entry.path(), &target).expect("copying the file");
            written += 1;
        }
        written
    }

    /// The encoding is what stops a report from becoming a mail header.
    ///
    /// A subject carrying a newline could otherwise close the `subject` field
    /// and open a `bcc`, which would turn a support address into a way of
    /// sending someone else's diagnostics somewhere else. The body is written
    /// by the user and the errors come from paths on their disk, so neither is
    /// trusted enough to interpolate raw.
    #[test]
    fn the_mailto_encoding_leaves_no_way_to_inject_a_header() {
        // Newlines and carriage returns are the injection vector.
        assert_eq!(percent_encode("a\r\nbcc:someone"), "a%0D%0Abcc%3Asomeone");

        // The separators of the URL itself must not survive either.
        assert_eq!(percent_encode("a&b=c?d"), "a%26b%3Dc%3Fd");

        // Only the unreserved set of RFC 3986 goes through untouched.
        assert_eq!(percent_encode("Az0-_.~"), "Az0-_.~");

        // Non-ASCII is encoded byte by byte, so an accented folder name in a
        // path does not break the URL.
        assert_eq!(percent_encode("città"), "citt%C3%A0");

        // A space is not unreserved: left raw it would truncate the argument.
        assert_eq!(percent_encode("due parole"), "due%20parole");
    }

    #[test]
    fn refuses_an_unrecognised_source() {
        let temp = TempDir::new("ignota");
        let file = temp.path().join("note.txt");
        write(&file, "contenuto qualsiasi");

        // A file that is not a Takeout archive must not be accepted.
        assert!(!zip_handler::is_takeout_archive(&file));

        // A folder with no known sections produces a notice, not an error.
        let summary = analyze_folder(temp.path()).expect("analisi folder vuota");
        assert!(summary.sections.is_empty());
        assert_eq!(summary.warnings.len(), 1);
    }
}
