// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Open Takeout Hub: elaborazione locale degli export Google Takeout.
//!
//! Regola architetturale del progetto: nessun modulo apre connessioni di rete.
//! Non ci sono client HTTP nel grafo delle dipendenze, non c'è telemetria, non
//! c'è updater automatico. I dati letti restano sul disco dell'utente e in
//! memoria per la durata della sessione.

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

/// Quanti record di esempio restituire al frontend per le anteprime.
const SAMPLE_SIZE: usize = 25;
/// Tetto agli elenchi lunghi (duplicati, segnaposto, file più grandi).
const MAX_ITEMS: usize = 50;

/// Nome dell'evento di avanzamento ascoltato dal frontend.
const PROGRESS_EVENT: &str = "takeout://progress";

/// Nome mostrato all'utente, distinto dal nome del pacchetto Cargo.
const APP_NAME: &str = "Open Takeout Hub";

/// Identificativo della voce di menu che apre la guida.
const MENU_HELP_ID: &str = "guida";

/// Evento con cui il menu chiede al frontend di mostrare la guida.
const SHOW_HELP_EVENT: &str = "takeout://mostra-guida";

/// Costruisce la barra dei menu.
///
/// Tauri saprebbe generarne una predefinita, ma le voci "About" e "Hide" di
/// macOS prenderebbero il nome dal processo, che durante lo sviluppo è il nome
/// dell'eseguibile (`open-takeout-hub`) perché non esiste ancora un bundle
/// `.app` con il suo `CFBundleName`. Dichiarandole a mano il nome è corretto sia
/// in sviluppo sia nel pacchetto distribuito, e le voci sono in italiano.
fn build_menu<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};

    let about = AboutMetadata {
        name: Some(APP_NAME.to_string()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        authors: Some(vec![env!("CARGO_PKG_AUTHORS").to_string()]),
        copyright: Some("Copyright (C) 2026 SkapaCraft".to_string()),
        license: Some("GPL-3.0-or-later".to_string()),
        // Il sito resta nei commenti come testo. Il campo `website` diventa un
        // collegamento cliccabile nella finestra di sistema di alcune
        // piattaforme, e questa applicazione non apre indirizzi.
        comments: Some(format!(
            "Elabora i tuoi export Google Takeout in locale.\n{}",
            env!("CARGO_PKG_HOMEPAGE")
        )),
        ..Default::default()
    };

    let guida = MenuItem::with_id(
        app,
        MENU_HELP_ID,
        "Guida di Open Takeout Hub",
        true,
        None::<&str>,
    )?;

    let modifica = Submenu::with_items(
        app,
        "Modifica",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("Annulla"))?,
            &PredefinedMenuItem::redo(app, Some("Ripristina"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("Taglia"))?,
            &PredefinedMenuItem::copy(app, Some("Copia"))?,
            &PredefinedMenuItem::paste(app, Some("Incolla"))?,
            &PredefinedMenuItem::select_all(app, Some("Seleziona tutto"))?,
        ],
    )?;

    let finestra = Submenu::with_items(
        app,
        "Finestra",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("Riduci a icona"))?,
            &PredefinedMenuItem::fullscreen(app, Some("Schermo intero"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, Some("Chiudi finestra"))?,
        ],
    )?;

    let aiuto = Submenu::with_items(app, "Aiuto", true, &[&guida])?;

    #[cfg(target_os = "macos")]
    {
        let applicazione = Submenu::with_items(
            app,
            APP_NAME,
            true,
            &[
                &PredefinedMenuItem::about(
                    app,
                    Some(&format!("Informazioni su {APP_NAME}")),
                    Some(about),
                )?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, Some("Servizi"))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, Some(&format!("Nascondi {APP_NAME}")))?,
                &PredefinedMenuItem::hide_others(app, Some("Nascondi altre"))?,
                &PredefinedMenuItem::show_all(app, Some("Mostra tutte"))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, Some(&format!("Esci da {APP_NAME}")))?,
            ],
        )?;
        Menu::with_items(app, &[&applicazione, &modifica, &finestra, &aiuto])
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Fuori da macOS non esiste il menu dell'applicazione: "Informazioni" e
        // "Esci" vanno sotto File, dove gli utenti li cercano.
        let file = Submenu::with_items(
            app,
            "File",
            true,
            &[
                &PredefinedMenuItem::about(
                    app,
                    Some(&format!("Informazioni su {APP_NAME}")),
                    Some(about),
                )?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, Some("Esci"))?,
            ],
        )?;
        Menu::with_items(app, &[&file, &modifica, &finestra, &aiuto])
    }
}

/// Intervallo minimo tra due eventi di avanzamento.
///
/// Emetterne uno per file significherebbe decine di migliaia di messaggi IPC e
/// altrettanti render React: la finestra si impunta proprio mentre mostra che
/// sta lavorando. A 80 ms l'occhio vede un avanzamento continuo e il thread di
/// rendering resta libero.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(80);

/// Costruisce il sink che inoltra l'avanzamento al webview, con throttling.
///
/// Gli eventi di inizio e fine passano sempre: sono quelli che fanno comparire
/// e sparire la barra, e perderli lascerebbe la UI in uno stato sbagliato.
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

        // Un errore di emissione significa finestra chiusa: l'elaborazione può
        // proseguire e terminare da sola, non è il caso di abortirla.
        let _ = app.emit(PROGRESS_EVENT, &progress);
    }
}

/// Esegue lavoro bloccante fuori dal runtime async, per non congelare la UI.
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
// Riconoscimento della sorgente
// ---------------------------------------------------------------------------

/// Individua la radice reale del Takeout.
///
/// L'utente può trascinare la cartella `Takeout/` oppure la cartella che la
/// contiene: normalizziamo i due casi.
fn resolve_takeout_root(path: &Path) -> PathBuf {
    let nested = path.join("Takeout");
    if nested.is_dir() {
        return nested;
    }
    path.to_path_buf()
}

/// Conta file e byte di una sottocartella.
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

/// Analizza una cartella Takeout già estratta.
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
        // Le cartelle nascoste non fanno parte dell'export.
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

/// Analizza un archivio `takeout-*.zip` senza estrarlo.
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

    // Dentro l'archivio le sezioni sono le cartelle al secondo livello, sotto
    // `Takeout/`. Le elenchiamo senza estrarre, quindi senza dimensioni per
    // sezione: quelle richiederebbero una seconda passata completa.
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
// Comandi esposti al frontend
// ---------------------------------------------------------------------------

/// Carica una sorgente (cartella o archivio) e ne restituisce il riepilogo.
#[tauri::command]
fn load_source(path: String, state: State<'_, AppState>) -> Result<SourceSummary> {
    let path = PathBuf::from(path);
    app_state::require_existing(&path)?;

    let summary = if path.is_dir() {
        analyze_folder(&path)?
    } else if zip_handler::is_takeout_archive(&path) {
        analyze_archive(&path)?
    } else {
        return Err(TakeoutError::Archive(format!(
            "{} non è una cartella Takeout né un archivio takeout-*.zip",
            path.display()
        )));
    };

    state.set_source(LoadedSource {
        root: summary.root.clone(),
        summary: summary.clone(),
    })?;

    Ok(summary)
}

/// Risolve il percorso su cui operare.
///
/// I comandi di analisi accettano un percorso esplicito (una sezione scelta
/// nella UI) oppure nessun percorso, e in quel caso lavorano sull'intera
/// sorgente caricata.
fn target_path(path: Option<String>, state: &State<'_, AppState>) -> Result<PathBuf> {
    match path {
        Some(path) => Ok(PathBuf::from(path)),
        None => state.root(),
    }
}

/// Riepilogo della sorgente attualmente caricata.
#[tauri::command]
fn current_source(state: State<'_, AppState>) -> Result<SourceSummary> {
    state.summary()
}

/// Dimentica la sorgente corrente.
#[tauri::command]
fn close_source(state: State<'_, AppState>) -> Result<()> {
    state.clear()
}

/// Ispeziona un archivio senza estrarlo.
#[tauri::command]
fn inspect_archive(path: String) -> Result<ArchiveSummary> {
    zip_handler::inspect(Path::new(&path))
}

/// Elenca le prime voci di un archivio.
#[tauri::command]
fn list_archive_entries(path: String, limit: Option<usize>) -> Result<Vec<ArchiveEntry>> {
    zip_handler::list_entries(Path::new(&path), limit.unwrap_or(MAX_ITEMS))
}

/// Estrae un archivio nella destinazione indicata dall'utente.
#[tauri::command]
fn extract_archive(path: String, destination: String) -> Result<ExtractReport> {
    zip_handler::extract(Path::new(&path), Path::new(&destination))
}

/// Individua tutti gli archivi che compongono lo stesso export.
#[tauri::command]
fn discover_archive_series(path: String) -> Result<ArchiveSeries> {
    zip_handler::discover_series(Path::new(&path))
}

/// Estrae l'intera serie di archivi in un unico albero.
///
/// È il comando che l'interfaccia usa davvero: partendo da un archivio
/// qualsiasi ricostruisce la serie e la unisce, invece di costringere l'utente
/// a estrarre a mano dodici file uno sopra l'altro.
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

/// Analizza una cartella di Google Foto.
#[tauri::command]
fn scan_photos(path: Option<String>, state: State<'_, AppState>) -> Result<PhotoScanReport> {
    exif_parser::scan_directory(&target_path(path, &state)?, SAMPLE_SIZE)
}

/// Ripara data e coordinate dei media, secondo la modalità richiesta.
///
/// Il comando è asincrono e delega a un thread di lavoro: su decine di
/// migliaia di foto un comando sincrono bloccherebbe la finestra per minuti.
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

/// Analizza l'export Contatti.
#[tauri::command]
fn scan_contacts(path: Option<String>, state: State<'_, AppState>) -> Result<ContactsReport> {
    contacts::scan_directory(&target_path(path, &state)?, SAMPLE_SIZE)
}

/// Analizza l'export Drive.
#[tauri::command]
fn scan_drive(path: Option<String>, state: State<'_, AppState>) -> Result<DriveReport> {
    drive::scan_directory(&target_path(path, &state)?, MAX_ITEMS)
}

/// Calcola il piano di pulizia senza toccare nulla.
///
/// La deduplica legge il contenuto dei file candidati, quindi su un export
/// grande è un'operazione lunga: va in background come le altre.
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

/// Esegue la pulizia: albero pulito altrove, oppure quarantena reversibile.
#[tauri::command]
async fn clean_drive(app: AppHandle, path: String, options: CleanOptions) -> Result<CleanReport> {
    in_background(move || {
        let sink = progress_emitter(app);
        drive::clean(Path::new(&path), &options, &sink)
    })
    .await
}

/// Sposta i sidecar il cui contenuto è ormai dentro ai file.
///
/// Ultimo passo di una riparazione riuscita, non una pulizia: sposta solo i
/// JSON che non sono più l'unica copia di qualcosa, e scrive il registro che
/// permette di annullare.
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

/// Rimette al loro posto i file spostati in quarantena.
#[tauri::command]
async fn restore_quarantine(manifest: String) -> Result<RestoreReport> {
    in_background(move || drive::restore_quarantine(Path::new(&manifest))).await
}

/// Ricostruisce la struttura di un export Google Foto: album, cartelle per
/// anno e versioni modificate.
#[tauri::command]
async fn scan_albums(path: String) -> Result<AlbumIndex> {
    in_background(move || albums::build_index(Path::new(&path), MAX_ITEMS)).await
}

/// Scrive il manifest degli album, da fare prima di deduplicare.
#[tauri::command]
fn export_album_manifest(path: String, destination: String) -> Result<ExportReport> {
    albums::export_manifest(Path::new(&path), Path::new(&destination))
}

/// Analizza l'export Calendario.
#[tauri::command]
fn scan_calendar(path: Option<String>, state: State<'_, AppState>) -> Result<CalendarReport> {
    calendar::scan_directory(&target_path(path, &state)?, SAMPLE_SIZE)
}

/// Scrive un vCard 3.0 pulito e deduplicato.
#[tauri::command]
fn export_contacts(path: String, destination: String) -> Result<ExportReport> {
    contacts::export_vcf(Path::new(&path), Path::new(&destination))
}

/// Scrive un iCalendar 2.0 pulito e deduplicato.
#[tauri::command]
fn export_calendar(path: String, destination: String) -> Result<ExportReport> {
    calendar::export_ics(Path::new(&path), Path::new(&destination))
}

/// Conti sullo spazio, per scegliere la modalità prima di cominciare.
///
/// Su una libreria grande la domanda non è se l'operazione funziona, ma se ci
/// sta: la copia riparata duplica tutto, la riscrittura sul posto no.
#[tauri::command]
async fn estimate_space(source: String, destination: String) -> Result<SpaceEstimate> {
    in_background(move || compute_space(Path::new(&source), Path::new(&destination))).await
}

/// Margine richiesto oltre ai byte da scrivere, come in `require_free_space`.
const MARGINE_DISCO: f64 = 1.10;

/// Calcola i conti sullo spazio e le tranche in cui dividere il lavoro.
///
/// Vive qui e non in `app_state` perché per distinguere una cartella per anno
/// da un album serve `albums`, e chiamarlo dallo stato condiviso rovescerebbe
/// la direzione delle dipendenze.
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

    // Prima passata: i nomi dei media che stanno in una cartella per anno.
    // Servono per sapere quali foto di un album esistono soltanto lì.
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
                    if let Some(nome) = file.file_name().to_str() {
                        nelle_annate.insert(nome.to_string());
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

            // Solo per gli album ha senso chiedersi se la foto esista altrove.
            if is_album && exif_parser::is_media_file(file.path()) {
                let assente = file
                    .file_name()
                    .to_str()
                    .is_none_or(|nome| !nelle_annate.contains(nome));
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
        // Sul posto si lavora su un file per volta e per thread.
        needed_in_place: largest.saturating_mul(4).max(64 * 1024 * 1024),
        subfolders,
    })
}

/// Dati identificativi dell'applicazione, per la guida.
#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo::default()
}

/// Percorso del file di preferenze, nella cartella di configurazione di sistema.
fn preferences_path(app: &AppHandle) -> Result<PathBuf> {
    use tauri::Manager;

    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| TakeoutError::Metadata(format!("cartella di configurazione: {e}")))?;
    Ok(dir.join("preferences.json"))
}

/// Legge le preferenze. Un file assente significa valori predefiniti.
#[tauri::command]
fn read_preferences(app: AppHandle) -> Result<Preferences> {
    let path = preferences_path(&app)?;
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Ok(Preferences::default());
    };
    // Un file corrotto non deve impedire l'avvio: si riparte dai predefiniti.
    Ok(serde_json::from_str(&content).unwrap_or_default())
}

/// Salva le preferenze.
#[tauri::command]
fn write_preferences(app: AppHandle, preferences: Preferences) -> Result<()> {
    let path = preferences_path(&app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TakeoutError::io(parent, e))?;
    }
    let json = serde_json::to_string_pretty(&preferences)
        .map_err(|e| TakeoutError::Metadata(e.to_string()))?;
    std::fs::write(&path, json).map_err(|e| TakeoutError::io(&path, e))?;
    trace_dev!("preferenze salvate in {}", path.display());
    Ok(())
}

/// Mostra un percorso nel gestore file del sistema.
///
/// Non usa `tauri-plugin-opener`, che resta vietato in `deny.toml` perché sa
/// aprire anche URL nel browser. Qui il programma invocato è fisso, l'unico
/// argomento è un percorso che deve già esistere, e non passa da una shell:
/// non c'è stringa di comando che l'utente possa influenzare.
///
/// Su Linux si apre la cartella e non il file: `xdg-open` su un file lo
/// aprirebbe con l'applicazione predefinita, che è un'altra cosa dal mostrarlo.
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
        .map_err(|e| TakeoutError::Task(format!("gestore file non avviato: {e}")))?;
    Ok(())
}

/// Dichiarazione del profilo privacy, mostrata nella UI.
#[tauri::command]
fn privacy_report() -> PrivacyReport {
    PrivacyReport::default()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Unico plugin registrato: il selettore file di sistema. Nessun opener
        // di URL, nessun updater, nessun canale verso l'esterno.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .setup(|app| {
            let menu = build_menu(app.handle())?;
            app.set_menu(menu)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id() == MENU_HELP_ID {
                // Il menu non conosce lo stato della UI: si limita a chiedere,
                // e il frontend decide come mostrare la guida.
                let _ = app.emit(SHOW_HELP_EVENT, ());
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
        ])
        .run(tauri::generate_context!())
        .expect("errore durante l'avvio dell'applicazione Tauri");
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app_state::testing::{write_bytes, write_file as write, TempDir, MINIMAL_JPEG};

    /// Costruisce un Takeout sintetico con le tre sezioni analizzabili.
    fn build_fixture(root: &Path) {
        let takeout = root.join("Takeout");

        // Google Foto: un JPEG reale con il sidecar che porta data e posizione.
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

        // Contatti: due schede, una duplicata per email.
        write(
            &takeout.join("Contatti").join("contatti.vcf"),
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Mario Rossi\r\nEMAIL:mario@example.com\r\nEND:VCARD\r\n\
             BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Mario Rossi\r\nEMAIL:MARIO@example.com\r\nTEL:+39 320 1234567\r\nEND:VCARD\r\n\
             BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Giulia Bianchi\r\nEMAIL:giulia@example.com\r\nEND:VCARD\r\n",
        );

        // Drive: un documento reale, un segnaposto, due copie identiche.
        let drive = takeout.join("Drive");
        write(&drive.join("relazione.docx"), "contenuto documento");
        write(
            &drive.join("appunti.gdoc"),
            r#"{"url": "https://docs.google.com/open?id=abc123", "doc_id": "abc123"}"#,
        );
        write(&drive.join("a").join("copia.txt"), "stesso contenuto");
        write(&drive.join("b").join("copia.txt"), "stesso contenuto");
    }

    #[test]
    fn riconosce_le_sezioni_di_un_takeout_sintetico() {
        let temp = TempDir::new("sezioni");
        build_fixture(temp.path());

        // La radice viene passata come cartella contenitore: deve scendere in `Takeout/`.
        let summary = analyze_folder(temp.path()).expect("analisi della cartella");

        assert_eq!(summary.kind, SourceKind::Folder);
        assert!(summary.root.ends_with("Takeout"));
        assert_eq!(summary.sections.len(), 3);
        assert!(summary.warnings.is_empty());
        // 2 in Google Foto (media + sidecar), 1 in Contatti, 4 in Drive.
        assert_eq!(summary.file_count, 7);

        let sections: Vec<TakeoutSection> = summary.sections.iter().map(|s| s.section).collect();
        assert!(sections.contains(&TakeoutSection::GooglePhotos));
        assert!(sections.contains(&TakeoutSection::Contacts));
        assert!(sections.contains(&TakeoutSection::Drive));
    }

    #[test]
    fn riconcilia_le_date_delle_foto_dal_sidecar() {
        let temp = TempDir::new("foto");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");

        let report = exif_parser::scan_directory(&photos, SAMPLE_SIZE).expect("scansione foto");
        assert_eq!(report.media_count, 1);
        assert_eq!(report.with_sidecar, 1);
        assert_eq!(report.with_exif_date, 0, "il file non ha EXIF");
        assert_eq!(report.without_exif, 1);
        assert_eq!(report.needs_repair, 1, "la data esiste solo nel sidecar");
        assert_eq!(report.with_geo, 1);

        let media = &report.sample[0];
        assert_eq!(media.taken_at_source, exif_parser::MetadataSource::Sidecar);
        assert_eq!(
            media.resolved_taken_at.expect("data risolta").timestamp(),
            1_577_880_000
        );

        // La simulazione conta i candidati senza toccare il disco.
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
            "la simulazione non deve toccare i byte"
        );
    }

    #[test]
    fn la_modalita_copia_non_tocca_gli_originali() {
        let temp = TempDir::new("copia");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");
        let uscita = temp.path().join("riparate");

        let originale_prima = std::fs::read(photos.join("IMG_0001.JPG")).expect("lettura");
        let mtime_prima = std::fs::metadata(photos.join("IMG_0001.JPG"))
            .and_then(|m| m.modified())
            .expect("mtime");

        let report = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                output_root: Some(uscita.clone()),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("copia riparata");

        assert_eq!(report.candidates, 1);
        assert!(report.failures.is_empty(), "{:?}", report.failures);

        // L'originale deve essere identico, byte per byte e come data.
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

        // La copia esiste e porta la data di scatto.
        let copia = uscita.join("IMG_0001.JPG");
        assert!(copia.is_file(), "la copia deve essere prodotta");
        let seconds = std::fs::metadata(&copia)
            .and_then(|m| m.modified())
            .expect("mtime copia")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("epoch")
            .as_secs();
        assert_eq!(seconds, 1_577_880_000);
        assert_eq!(report.file_times_written, 1);
        assert_eq!(report.exif_written, 1);

        // Prova del nove: i tag scritti devono essere rileggibili, e il round
        // trip attraverso gradi/primi/secondi deve conservare le coordinate.
        let riletto = exif_parser::read_exif(&copia).expect("rilettura EXIF");
        assert_eq!(
            riletto.taken_at.expect("data scritta").timestamp(),
            1_577_880_000,
            "la data di scatto deve essere ora dentro al file"
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

        // Il file riparato resta un JPEG valido, non un contenitore corrotto.
        let bytes = std::fs::read(&copia).expect("lettura copia");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "firma JPEG intatta");

        // Prova decisiva sul fuso: il round trip qui sopra tornerebbe corretto
        // anche scrivendo UTC, quindi va guardata la stringa dentro al file.
        // Le coordinate del fixture sono a Milano e la data è di gennaio: sul
        // posto l'orologio segnava le 13, non le 12 di UTC.
        let grezzo = String::from_utf8_lossy(&bytes);
        assert!(
            grezzo.contains("2020:01:01 13:00:00"),
            "DateTimeOriginal deve portare l'ora locale del luogo"
        );
        assert!(
            grezzo.contains("+01:00"),
            "l'offset va dichiarato accanto alla data"
        );
    }

    #[test]
    fn la_copia_include_anche_i_formati_senza_exif() {
        let temp = TempDir::new("copia-video");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");
        let uscita = temp.path().join("riparate");

        // Un video con il suo sidecar: non sappiamo scriverne l'EXIF, ma
        // l'albero di uscita deve restare completo, altrimenti l'utente si
        // ritrova una copia con dentro solo metà dei ricordi.
        write(&photos.join("VID_0001.MP4"), "contenuto video finto");
        write(
            &photos.join("VID_0001.MP4.json"),
            r#"{"photoTakenTime": { "timestamp": "1577880000" }}"#,
        );

        let report = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                output_root: Some(uscita.clone()),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("copia riparata");

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.skipped_unsupported, 1, "il video non ha EXIF");
        assert!(
            uscita.join("VID_0001.MP4").is_file(),
            "il video deve comunque essere copiato nell'albero di uscita"
        );
        assert!(uscita.join("IMG_0001.JPG").is_file());

        // Senza EXIF scritto la data vive solo nell'mtime, che è fragile: il
        // sidecar deve seguire il file, altrimenti l'unica fonte durevole
        // resta indietro nella cartella di origine.
        assert_eq!(report.sidecars_copied, 1);
        assert!(
            uscita.join("VID_0001.MP4.json").is_file(),
            "il sidecar del video va conservato accanto alla copia"
        );
        // Per il JPEG l'EXIF è stato scritto dentro al file: il sidecar
        // sarebbe una duplicazione e non viene riportato.
        assert!(!uscita.join("IMG_0001.JPG.json").exists());

        // Anche senza EXIF, la data del file va allineata.
        let seconds = std::fs::metadata(uscita.join("VID_0001.MP4"))
            .and_then(|m| m.modified())
            .expect("mtime video")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("epoch")
            .as_secs();
        assert_eq!(seconds, 1_577_880_000);
    }

    /// La modalità che riscrive gli originali è l'unica irreversibile, quindi
    /// è quella che merita il test più severo: deve scrivere i tag e allineare
    /// la data **senza** alterare un solo byte dell'immagine.
    #[test]
    fn la_modalita_in_place_riscrive_senza_toccare_i_pixel() {
        let temp = TempDir::new("in-place");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");
        let originale = photos.join("IMG_0001.JPG");

        /// Estrae i dati compressi dell'immagine principale, scavalcando i
        /// segmenti di intestazione. La miniatura dentro l'EXIF ha un proprio
        /// marcatore SOS, quindi cercare il primo `FFDA` darebbe il risultato
        /// sbagliato.
        fn scan_data(path: &Path) -> Vec<u8> {
            let bytes = std::fs::read(path).expect("lettura JPEG");
            let mut i = 2; // dopo SOI
            while i < bytes.len() - 1 && bytes[i] == 0xFF {
                let marker = bytes[i + 1];
                // TEM, RST0-7 e SOI non hanno un campo lunghezza.
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
            panic!("marcatore SOS non trovato");
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
        // In questa modalità non si copia nulla: il sidecar resta dov'è.
        assert_eq!(report.sidecars_copied, 0);

        // Il file è stato riscritto sul posto, non duplicato altrove.
        assert!(originale.is_file());
        assert!(!photos.join(".oth-tmp-IMG_0001.JPG").exists());

        // I dati immagine devono essere identici byte per byte.
        assert_eq!(
            scan_data(&originale),
            pixel_prima,
            "la riscrittura in place ha alterato i pixel"
        );

        // I tag sono davvero dentro l'originale, ora.
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
    fn dispone_luscita_in_ordine_cronologico() {
        let temp = TempDir::new("layout");
        let photos = temp.path().join("foto");
        let uscita = temp.path().join("cronologico");

        // Due foto con lo stesso nome in cartelle diverse, come capita quando
        // la stessa immagine sta in un album e in una cartella per anno, più
        // una senza alcuna data ricavabile.
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
                output_root: Some(uscita.clone()),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("uscita cronologica");
        assert!(report.failures.is_empty(), "{:?}", report.failures);

        // 2020-01-01 e 2020-03-01 finiscono in mesi diversi, quindi nessuna
        // collisione nonostante il nome identico.
        assert!(uscita.join("2020/01/IMG_1.JPG").is_file());
        assert!(uscita.join("2020/03/IMG_1.JPG").is_file());

        // Chi non ha data non viene infilato in un mese inventato.
        assert!(uscita.join("senza-data/senza.JPG").is_file());
    }

    #[test]
    fn non_sovrascrive_i_nomi_uguali_nella_stessa_cartella() {
        let temp = TempDir::new("collisioni-layout");
        let photos = temp.path().join("foto");
        let uscita = temp.path().join("piatto");

        // Stesso nome, stessa data, cartelle diverse: nel layout piatto
        // finirebbero sullo stesso percorso.
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
                output_root: Some(uscita.clone()),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("uscita piatta");
        assert!(report.failures.is_empty(), "{:?}", report.failures);

        // Tutte e tre devono sopravvivere, con un contatore progressivo.
        let prodotti = std::fs::read_dir(&uscita)
            .expect("lettura uscita")
            .flatten()
            .count();
        assert_eq!(prodotti, 3, "nessuna foto deve essere sovrascritta");
        assert!(uscita.join("IMG_1.JPG").is_file());
        assert!(uscita.join("IMG_1 (2).JPG").is_file());
        assert!(uscita.join("IMG_1 (3).JPG").is_file());
    }

    /// Su una libreria da decine di gigabyte la copia riparata ne richiede
    /// altrettanti: se il disco si riempie a metà, l'albero di uscita sembra
    /// completo e non lo è. Meglio rifiutare prima di cominciare.
    /// Lavorando a tranche la tentazione è riparare solo le cartelle per anno,
    /// perché gli album sono quasi tutti copie. Quel "quasi" è il punto: una
    /// foto che sta soltanto in un album verrebbe lasciata indietro, e chi
    /// guarda il risultato non se ne accorgerebbe.
    #[test]
    fn distingue_le_annate_dagli_album_e_conta_le_foto_uniche() {
        let temp = TempDir::new("tranche");
        let foto = temp.path().join("Google Foto");

        write_bytes(&foto.join("Foto da 2020").join("IMG_1.JPG"), MINIMAL_JPEG);
        write_bytes(&foto.join("Foto da 2020").join("IMG_2.JPG"), MINIMAL_JPEG);
        // Album con una copia e una foto che non sta da nessun'altra parte.
        write_bytes(&foto.join("Vacanze").join("IMG_1.JPG"), MINIMAL_JPEG);
        write_bytes(&foto.join("Vacanze").join("SOLO_QUI.JPG"), MINIMAL_JPEG);
        // Album fatto di sole copie: saltarlo non costa nulla.
        write_bytes(&foto.join("Compleanno").join("IMG_2.JPG"), MINIMAL_JPEG);

        let stima = compute_space(&foto, temp.path()).expect("stima");

        let annata = stima
            .subfolders
            .iter()
            .find(|f| f.name == "Foto da 2020")
            .expect("annata");
        assert!(annata.is_year && !annata.is_album);
        assert_eq!(annata.unique_here, 0, "sulle annate la domanda non si pone");

        let vacanze = stima
            .subfolders
            .iter()
            .find(|f| f.name == "Vacanze")
            .expect("album");
        assert!(vacanze.is_album && !vacanze.is_year);
        assert_eq!(
            vacanze.unique_here, 1,
            "SOLO_QUI non esiste in nessuna annata: saltare questo album la perderebbe"
        );

        let compleanno = stima
            .subfolders
            .iter()
            .find(|f| f.name == "Compleanno")
            .expect("album");
        assert_eq!(
            compleanno.unique_here, 0,
            "solo copie: si può saltare senza perdere niente"
        );
    }

    #[test]
    fn rifiuta_se_manca_lo_spazio_sulla_destinazione() {
        let temp = TempDir::new("spazio");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");

        // Uno spazio richiesto assurdo non può essere disponibile da nessuna
        // parte, quindi il controllo deve scattare.
        let esito = app_state::require_free_space(temp.path(), u64::MAX / 2);
        assert!(esito.is_err(), "va rifiutato prima di scrivere");
        let messaggio = esito.unwrap_err().to_string();
        assert!(
            messaggio.contains("not enough space"),
            "il messaggio deve dire cosa manca: {messaggio}"
        );

        // Con una richiesta plausibile invece passa, e la riparazione procede.
        app_state::require_free_space(temp.path(), 1024).expect("mille byte ci stanno");

        let report = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                output_root: Some(temp.path().join("uscita")),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("riparazione");
        assert!(report.failures.is_empty());
    }

    #[test]
    fn rifiuta_una_destinazione_dentro_la_sorgente() {
        let temp = TempDir::new("ricorsione");
        build_fixture(temp.path());
        let photos = temp.path().join("Takeout").join("Google Foto");

        let esito = exif_parser::apply_metadata(
            &photos,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                output_root: Some(photos.join("uscita")),
                ..Default::default()
            },
            &app_state::no_progress,
        );

        assert!(esito.is_err(), "una destinazione annidata va rifiutata");
    }

    #[test]
    fn deduplica_i_contatti() {
        let temp = TempDir::new("contatti");
        build_fixture(temp.path());

        let report =
            contacts::scan_directory(&temp.path().join("Takeout").join("Contatti"), SAMPLE_SIZE)
                .expect("scansione contatti");

        assert_eq!(report.total, 3);
        assert_eq!(report.duplicates, 1, "le due schede di Mario coincidono");
        assert_eq!(report.unique, 2);
        assert_eq!(report.with_email, 3);

        // La fusione non deve perdere il telefono presente solo nel duplicato.
        let mario = report
            .sample
            .iter()
            .find(|c| c.display_name.as_deref() == Some("Mario Rossi"))
            .expect("contatto fuso");
        assert_eq!(mario.phones.len(), 1);
    }

    #[test]
    fn segnala_i_segnaposto_e_i_duplicati_di_drive() {
        let temp = TempDir::new("drive");
        build_fixture(temp.path());

        let report = drive::scan_directory(&temp.path().join("Takeout").join("Drive"), MAX_ITEMS)
            .expect("scansione drive");

        assert_eq!(report.file_count, 4);
        assert_eq!(report.placeholder_count, 1);
        assert_eq!(
            report.placeholders[0].target_url.as_deref(),
            Some("https://docs.google.com/open?id=abc123")
        );
        assert_eq!(report.duplicate_groups.len(), 1);
        assert_eq!(report.duplicate_groups[0].paths.len(), 2);
        assert!(!report.warnings.is_empty(), "il segnaposto va segnalato");
    }

    /// Misura il comportamento su una libreria di dimensioni realistiche.
    ///
    /// Escluso dalla CI perché genera decine di migliaia di file. Si lancia a
    /// mano, eventualmente scegliendo quante foto produrre:
    ///
    /// ```bash
    /// FOTO=50000 cargo test --release misura_su_libreria_grande -- --ignored --nocapture
    /// ```
    ///
    /// Va lanciato in release, e non per pignoleria: in debug il codice Rust è
    /// oltre un ordine di grandezza più lento, quindi i tempi non direbbero
    /// nulla di utile, e la diagnostica di sviluppo stamperebbe una riga per
    /// ogni file sommergendo il risultato.
    ///
    /// Vale la pena chiarire perché non serve un export da cento gigabyte.
    /// Quello che mette in difficoltà questo codice è il **numero di file**,
    /// non il numero di byte: cento gigabyte di video sono duecento file, cioè
    /// nulla. Con un JPEG da 889 byte se ne generano centomila in centocinquanta
    /// megabyte, ottenendo una prova più severa di una libreria reale della
    /// stessa consistenza.
    ///
    /// La misura che conta di più non è il tempo ma la dimensione dei report
    /// serializzati: sono ciò che attraversa il canale IPC verso l'interfaccia
    /// a ogni scansione, e un elenco senza tetto lì diventa megabyte di JSON.
    #[test]
    #[ignore = "genera decine di migliaia di file: si lancia a mano"]
    fn misura_su_libreria_grande() {
        use std::time::Instant;

        let foto_totali: usize = std::env::var("FOTO")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20_000);

        let temp = TempDir::new("scala");
        let root = temp.path().join("Takeout").join("Google Foto");

        // Struttura simile a un export vero: cartelle per anno, album che
        // ripetono una parte delle foto, versioni modificate, e una quota
        // senza data ricavabile.
        let anni = [2019, 2020, 2021, 2022, 2023];
        let album = [
            "Vacanze in Sicilia",
            "Compleanno di Anna",
            "Montagna 2021",
            "Matrimonio",
        ];

        let inizio = Instant::now();
        let mut nomi: Vec<(String, usize)> = Vec::with_capacity(foto_totali);

        /// Produce un JPEG valido ma diverso da tutti gli altri, mantenendo la
        /// stessa dimensione.
        ///
        /// I byte in coda dopo il marcatore di fine sono ignorati dai
        /// decodificatori, quindi il file resta leggibile. Serve a evitare che
        /// il fixture sia tutto identico: in quel caso l'hash li unirebbe in un
        /// unico gruppo enorme e la misura non direbbe nulla sul caso reale.
        /// Con dimensione uguale e contenuto diverso invece si ottiene lo
        /// scenario più oneroso per la deduplica, che deve leggere ogni file
        /// per scoprire che sono tutti distinti.
        fn jpeg_unico(indice: usize) -> Vec<u8> {
            let mut bytes = MINIMAL_JPEG.to_vec();
            bytes.extend_from_slice(&(indice as u64).to_le_bytes());
            bytes
        }

        for indice in 0..foto_totali {
            let anno = anni[indice % anni.len()];
            let mese = (indice % 12) + 1;
            let giorno = (indice % 28) + 1;
            let nome = format!("IMG_{anno}{mese:02}{giorno:02}_{:06}.JPG", indice % 240_000);
            let cartella = root.join(format!("Foto da {anno}"));

            write_bytes(&cartella.join(&nome), &jpeg_unico(indice));

            // Una foto su cinque resta senza sidecar: dovrà cavarsela con la
            // data dedotta dal nome.
            if indice % 5 != 0 {
                let istante = 1_577_880_000 + (indice as i64 * 37);
                // Una su tre ha le coordinate, quindi passa dalla conversione
                // di fuso orario, che è il percorso più costoso.
                let geo = if indice % 3 == 0 {
                    r#", "geoData": {"latitude": 45.4642, "longitude": 9.19, "altitude": 120.0}"#
                } else {
                    ""
                };
                write(
                    &cartella.join(format!("{nome}.supplemental-metadata.json")),
                    &format!(r#"{{"photoTakenTime": {{"timestamp": "{istante}"}}{geo}}}"#),
                );
            }

            // Una foto su venti ha una versione modificata accanto.
            if indice % 20 == 0 {
                let modificata = nome.replace(".JPG", "-modificato.JPG");
                // Una versione modificata ha pixel diversi: non è un duplicato.
                write_bytes(
                    &cartella.join(&modificata),
                    &jpeg_unico(indice + foto_totali),
                );
            }

            nomi.push((nome, indice));
        }

        // Un decimo delle foto compare anche in un album: è il caso che rende
        // necessario il manifest.
        // Questi sì che sono duplicati veri: copia identica della foto che sta
        // già nella cartella per anno.
        for (nome, indice) in nomi.iter().filter(|(_, i)| i % 10 == 0) {
            let scelto = album[indice % album.len()];
            write_bytes(&root.join(scelto).join(nome), &jpeg_unico(*indice));
        }

        let file_totali = WalkDir::new(&root)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .count();
        let byte_totali: u64 = WalkDir::new(&root)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();

        println!("\n=== libreria generata ===");
        println!("  foto:        {foto_totali}");
        println!("  file totali: {file_totali}");
        println!("  byte:        {:.1} MB", byte_totali as f64 / 1e6);
        println!("  generazione: {:.1} s", inizio.elapsed().as_secs_f64());

        /// Misura durata e peso del report serializzato, cioè quanto passa
        /// davvero dal canale IPC.
        fn misura<T: serde::Serialize>(nome: &str, lavoro: impl FnOnce() -> T) {
            let inizio = Instant::now();
            let esito = lavoro();
            let durata = inizio.elapsed();
            let json = serde_json::to_string(&esito).unwrap_or_default();
            println!(
                "  {nome:<22} {:>7.2} s   report {:>8.2} MB",
                durata.as_secs_f64(),
                json.len() as f64 / 1e6
            );
        }

        println!("\n=== operazioni ===");
        misura("scansione foto", || {
            exif_parser::scan_directory(&root, SAMPLE_SIZE).expect("scansione")
        });
        misura("indice album", || {
            albums::build_index(&root, MAX_ITEMS).expect("indice")
        });
        misura("piano di pulizia", || {
            drive::plan_clean(
                &root,
                &drive::CleanOptions::default(),
                MAX_ITEMS,
                &app_state::no_progress,
            )
            .expect("piano")
        });
        println!();
    }

    /// Misura il percorso dei byte veri: hashing, copia e soglia di riscrittura.
    ///
    /// È l'altra metà della prova di scala. Quella su centomila foto verifica
    /// il costo per file; questa verifica il costo per byte, che è dominato dal
    /// disco ma tocca due punti nostri: `little_exif` carica in memoria
    /// l'intero file da riscrivere, e i thread sono quattro, quindi il picco
    /// cresce con la dimensione dei singoli media.
    ///
    /// ```bash
    /// GB=2 cargo test --release misura_su_file_grandi -- --ignored --nocapture
    /// ```
    ///
    /// Il picco di memoria non è misurato da qui: si legge da fuori.
    ///
    /// ```bash
    /// /usr/bin/time -l cargo test --release misura_su_file_grandi -- --ignored --nocapture
    /// ```
    ///
    /// Attenzione a quale numero si guarda. Il `maximum resident set size` su
    /// macOS comprende le pagine dei file toccati, che appartengono alla cache
    /// del kernel e vengono recuperate sotto pressione: cresce con la quantità
    /// di dati letti e scritti, varia tra due esecuzioni identiche, e non dice
    /// nulla su quanto alloca il programma. Il dato che conta è
    /// `peak memory footprint`, che misura la memoria anonima: su due gigabyte
    /// e mezzo di media resta intorno ai cento megabyte, cioè entro il tetto
    /// che i quattro thread e la soglia di riscrittura dovrebbero garantire.
    #[test]
    #[ignore = "scrive qualche gigabyte: si lancia a mano"]
    fn misura_su_file_grandi() {
        use std::io::Write;
        use std::time::Instant;

        /// Dimensione dei media grandi ma ancora riscrivibili.
        ///
        /// Sotto la soglia di 128 MB, così passano davvero dalla riscrittura
        /// EXIF: è il caso che mette alla prova la memoria, perché ogni thread
        /// ne tiene una copia.
        const GRANDE: u64 = 64 * 1024 * 1024;
        /// Sopra la soglia: deve essere saltato, non riscritto.
        const ENORME: u64 = 200 * 1024 * 1024;

        let gigabyte: u64 = std::env::var("GB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2);
        let quanti = ((gigabyte * 1024 * 1024 * 1024) / GRANDE).max(2) as usize;

        let temp = TempDir::new("byte");
        let root = temp.path().join("Foto da 2024");
        let uscita = temp.path().join("riparate");

        /// Scrive un JPEG valido della dimensione richiesta.
        ///
        /// I byte dopo il marcatore di fine sono ignorati dai decodificatori,
        /// quindi il file resta leggibile. Il riempimento dipende dal seme, così
        /// due file della stessa dimensione hanno contenuto diverso e la
        /// deduplica deve leggerli per intero per scoprirlo.
        fn scrivi_grande(path: &Path, dimensione: u64, seme: u8) {
            std::fs::create_dir_all(path.parent().expect("genitore")).expect("cartelle");
            let file = std::fs::File::create(path).expect("creazione");
            let mut out = std::io::BufWriter::with_capacity(8 << 20, file);
            out.write_all(MINIMAL_JPEG).expect("intestazione");

            let blocco = vec![seme; 8 << 20];
            let mut scritti = MINIMAL_JPEG.len() as u64;
            while scritti < dimensione {
                let quanti = (dimensione - scritti).min(blocco.len() as u64) as usize;
                out.write_all(&blocco[..quanti]).expect("riempimento");
                scritti += quanti as u64;
            }
            out.flush().expect("flush");
        }

        let inizio = Instant::now();
        for indice in 0..quanti {
            let nome = format!("GRANDE_{indice:03}.JPG");
            scrivi_grande(&root.join(&nome), GRANDE, indice as u8);
            // Sidecar diversi tra loro, come in un export vero: il titolo
            // riporta il nome del file e l'istante cambia a ogni foto.
            write(
                &root.join(format!("{nome}.supplemental-metadata.json")),
                &format!(
                    r#"{{"title": "{nome}", "photoTakenTime": {{"timestamp": "{}"}}, "geoData": {{"latitude": 45.4642, "longitude": 9.19, "altitude": 120.0}}}}"#,
                    1_577_880_000_i64 + indice as i64
                ),
            );
        }
        // Una copia identica del primo: duplicato vero da trovare per contenuto.
        scrivi_grande(&temp.path().join("Album").join("GRANDE_000.JPG"), GRANDE, 0);
        // E uno oltre la soglia, che deve essere saltato dalla riscrittura.
        scrivi_grande(&root.join("ENORME.JPG"), ENORME, 200);

        let byte_totali: u64 = WalkDir::new(temp.path())
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        let generazione = inizio.elapsed().as_secs_f64();

        println!("\n=== libreria generata ===");
        println!("  media grandi: {quanti} da {} MB", GRANDE / 1024 / 1024);
        println!("  piu' uno da:  {} MB", ENORME / 1024 / 1024);
        println!("  totale:       {:.2} GB", byte_totali as f64 / 1e9);
        println!(
            "  scrittura:    {generazione:.1} s  ({:.0} MB/s)",
            byte_totali as f64 / 1e6 / generazione
        );

        println!("\n=== operazioni ===");

        let inizio = Instant::now();
        let piano = drive::plan_clean(
            temp.path(),
            &drive::CleanOptions::default(),
            MAX_ITEMS,
            &app_state::no_progress,
        )
        .expect("piano");
        let durata = inizio.elapsed().as_secs_f64();
        println!(
            "  deduplica     {durata:>7.2} s   letti {:.2} GB  ({:.0} MB/s)",
            piano.hashed_bytes as f64 / 1e9,
            piano.hashed_bytes as f64 / 1e6 / durata
        );
        assert_eq!(piano.duplicate_copies, 1, "la copia identica va trovata");

        let inizio = Instant::now();
        let report = exif_parser::apply_metadata(
            &root,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::CopyToOutput,
                output_root: Some(uscita.clone()),
                ..Default::default()
            },
            &app_state::no_progress,
        )
        .expect("riparazione");
        let durata = inizio.elapsed().as_secs_f64();
        let scritti: u64 = WalkDir::new(&uscita)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        println!(
            "  riparazione   {durata:>7.2} s   scritti {:.2} GB  ({:.0} MB/s)",
            scritti as f64 / 1e9,
            scritti as f64 / 1e6 / durata
        );

        println!("\n=== esito riparazione ===");
        println!("  EXIF scritti:        {}", report.exif_written);
        println!("  oltre la soglia:     {}", report.skipped_too_large);
        println!("  errori:              {}", report.failures.len());
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(
            report.exif_written, quanti,
            "i media sotto soglia si riscrivono"
        );
        assert_eq!(
            report.skipped_too_large, 1,
            "quello sopra soglia va saltato"
        );
        // Saltarlo non significa perderlo: la copia deve esserci comunque.
        assert!(
            uscita.join("ENORME.JPG").is_file(),
            "il file oltre soglia va copiato lo stesso"
        );
        println!();
    }

    /// Misura contatti e calendario su volumi realistici.
    ///
    /// Hanno un profilo opposto a quello delle foto: pochissimi file, ma
    /// grandi, e ciascuno viene letto interamente in memoria prima di essere
    /// interpretato. Il rischio non è il numero di aperture, è la dimensione
    /// del singolo file e il costo della deduplica, che confronta ogni scheda
    /// con quelle già viste.
    ///
    /// ```bash
    /// CONTATTI=20000 EVENTI=50000 cargo test --release misura_su_rubrica_grande -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "genera file di decine di megabyte: si lancia a mano"]
    fn misura_su_rubrica_grande() {
        use std::time::Instant;

        let contatti: usize = std::env::var("CONTATTI")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20_000);
        let eventi: usize = std::env::var("EVENTI")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50_000);

        let temp = TempDir::new("rubrica");
        let radice = temp.path().join("Takeout");

        let inizio = Instant::now();

        // Rubrica: un solo .vcf, come lo esporta Google. Una scheda su dieci è
        // un duplicato con la stessa email, il caso che la deduplica deve
        // riconoscere; una su sette ha una riga lunga, che obbliga il parser a
        // ricomporre il line folding.
        let mut vcf = String::with_capacity(contatti * 180);
        for indice in 0..contatti {
            // Una scheda su dieci ripete l'identità della precedente: è il caso
            // reale di chi ha salvato due volte lo stesso contatto.
            let chi = if indice % 10 == 9 { indice - 1 } else { indice };
            vcf.push_str("BEGIN:VCARD\r\nVERSION:3.0\r\n");
            vcf.push_str(&format!("FN:Persona Numero {chi}\r\n"));
            vcf.push_str(&format!("N:Numero;Persona{chi};;;\r\n"));
            vcf.push_str(&format!("EMAIL;TYPE=INTERNET:persona{chi}@example.com\r\n"));
            vcf.push_str(&format!(
                "TEL;TYPE=CELL:+39 320 {:07}\r\n",
                chi % 10_000_000
            ));
            if indice % 7 == 0 {
                // Riga spezzata secondo la regola del folding.
                vcf.push_str("NOTE:Appunto lungo che continua\r\n  sulla riga successiva\r\n");
            }
            vcf.push_str("END:VCARD\r\n");
        }
        write(&radice.join("Contatti").join("Tutti i contatti.vcf"), &vcf);

        // Calendario: cinque file, come cinque calendari dell'account. Un
        // evento su otto è ricorrente, uno su venti è di giornata intera, e
        // ciascuno porta proprietà proprietarie di Google da rimuovere.
        for calendario in 0..5 {
            let quanti = eventi / 5;
            let mut ics = String::with_capacity(quanti * 260);
            ics.push_str("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Google Inc//EN\r\n");
            for indice in 0..quanti {
                let giorno = (indice % 28) + 1;
                let mese = (indice % 12) + 1;
                let anno = 2018 + (indice % 8);
                ics.push_str("BEGIN:VEVENT\r\n");
                ics.push_str(&format!("UID:evento-{calendario}-{indice}@google.com\r\n"));
                if indice % 20 == 0 {
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
                ics.push_str(&format!("SUMMARY:Impegno numero {indice}\r\n"));
                ics.push_str("LOCATION:Ufficio\r\n");
                ics.push_str("X-GOOGLE-CONFERENCE:https://meet.google.com/abc-defg-hij\r\n");
                if indice % 8 == 0 {
                    ics.push_str("RRULE:FREQ=WEEKLY;COUNT=10\r\n");
                }
                ics.push_str(
                    "BEGIN:VALARM\r\nACTION:DISPLAY\r\nSUMMARY:Promemoria\r\nEND:VALARM\r\n",
                );
                ics.push_str("END:VEVENT\r\n");
            }
            ics.push_str("END:VCALENDAR\r\n");
            write(
                &radice
                    .join("Calendario")
                    .join(format!("calendario-{calendario}.ics")),
                &ics,
            );
        }

        let byte: u64 = WalkDir::new(&radice)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();

        println!("\n=== generati ===");
        println!("  contatti:  {contatti} in un solo .vcf");
        println!("  eventi:    {eventi} in 5 .ics");
        println!("  byte:      {:.1} MB", byte as f64 / 1e6);
        println!("  scrittura: {:.1} s", inizio.elapsed().as_secs_f64());

        fn misura<T: serde::Serialize>(nome: &str, lavoro: impl FnOnce() -> T) -> T {
            let inizio = Instant::now();
            let esito = lavoro();
            let json = serde_json::to_string(&esito).unwrap_or_default();
            println!(
                "  {nome:<24} {:>6.2} s   report {:>7.2} MB",
                inizio.elapsed().as_secs_f64(),
                json.len() as f64 / 1e6
            );
            esito
        }

        println!("\n=== operazioni ===");
        let rubrica = misura("scansione contatti", || {
            contacts::scan_directory(&radice.join("Contatti"), SAMPLE_SIZE).expect("contatti")
        });
        let agenda = misura("scansione calendario", || {
            calendar::scan_directory(&radice.join("Calendario"), SAMPLE_SIZE).expect("calendario")
        });

        let uscita = temp.path().join("uscita");
        misura("export vCard", || {
            contacts::export_vcf(&radice.join("Contatti"), &uscita.join("contatti.vcf"))
                .expect("export contatti")
        });
        misura("export iCalendar", || {
            calendar::export_ics(&radice.join("Calendario"), &uscita.join("calendario.ics"))
                .expect("export calendario")
        });

        println!("\n=== esito ===");
        println!(
            "  contatti: {} letti, {} unici, {} duplicati",
            rubrica.total, rubrica.unique, rubrica.duplicates
        );
        println!(
            "  eventi:   {} letti, {} unici, {} proprietà rimosse",
            agenda.total, agenda.unique, agenda.dropped_properties
        );

        // La deduplica deve riconoscere le schede ripetute, non contarle a caso.
        assert!(rubrica.duplicates > 0, "i duplicati vanno trovati");
        assert_eq!(rubrica.total, contatti);
        assert_eq!(agenda.total, eventi);
        // Gli allarmi non devono essere scambiati per eventi.
        assert!(
            agenda.sample.iter().all(|e| e
                .summary
                .as_deref()
                .is_some_and(|s| s.starts_with("Impegno"))),
            "il SUMMARY del VALARM non deve sovrascrivere quello dell'evento"
        );
        println!();
    }

    /// Estrae una serie multi-archivio presa dal disco e ne analizza il
    /// risultato.
    ///
    /// A differenza delle altre misure questa non genera nulla: lavora su
    /// archivi già esistenti, così da esercitare il percorso completo
    /// riconoscimento della serie, unione, scansione foto e album su materiale
    /// che non è stato costruito dagli stessi test che lo verificano.
    ///
    /// ```bash
    /// SERIE=~/Downloads/prova-multiarchivio USCITA=~/Downloads/estratto \
    ///   cargo test --release estrazione_di_una_serie_reale -- --ignored --nocapture
    /// ```
    ///
    /// `SERIE` può indicare la cartella che contiene gli archivi oppure uno
    /// qualsiasi di essi: il riconoscimento della serie parte da un archivio
    /// solo e trova gli altri da sé, ed è proprio quel comportamento che qui
    /// interessa provare. Senza `USCITA` l'estrazione finisce in una cartella
    /// temporanea, che viene rimossa alla fine; indicandola invece si può
    /// riusare l'albero estratto per le prove successive.
    ///
    /// Il test è escluso dalla CI perché dipende da file locali e, su una serie
    /// vera, scrive quanto pesa l'export.
    #[test]
    #[ignore = "richiede archivi sul disco: si lancia a mano con SERIE=..."]
    fn estrazione_di_una_serie_reale() {
        use std::time::Instant;

        let Ok(serie) = std::env::var("SERIE") else {
            println!("SERIE non impostata: niente da estrarre.");
            return;
        };
        let serie = PathBuf::from(serie);

        // Un archivio qualsiasi della serie basta: il resto lo trova da sé.
        let primo = if serie.is_dir() {
            let mut archivi: Vec<PathBuf> = std::fs::read_dir(&serie)
                .expect("lettura della cartella indicata")
                .filter_map(|v| v.ok().map(|v| v.path()))
                .filter(|p| zip_handler::is_takeout_archive(p))
                .collect();
            archivi.sort();
            archivi
                .into_iter()
                .next()
                .expect("nessun takeout-*.zip nella cartella indicata")
        } else {
            serie.clone()
        };

        let inizio = Instant::now();
        let trovata = zip_handler::discover_series(&primo).expect("riconoscimento della serie");
        println!(
            "\nserie riconosciuta partendo da {}",
            primo.file_name().unwrap_or_default().to_string_lossy()
        );
        println!(
            "  {} archivi, {:.2} GB compressi, mancanti: {:?}  ({:?})",
            trovata.archives.len(),
            trovata.total_compressed_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
            trovata.missing,
            inizio.elapsed()
        );
        assert!(
            trovata.missing.is_empty(),
            "la serie sul disco risulta incompleta"
        );

        // Con USCITA l'albero resta a disposizione, altrimenti sparisce.
        let scelta = std::env::var("USCITA").ok();
        let temporanea = scelta.is_none().then(|| TempDir::new("serie-reale"));
        let destinazione = match (&scelta, &temporanea) {
            (Some(percorso), _) => PathBuf::from(percorso),
            (None, Some(temp)) => temp.path().join("estratto"),
            (None, None) => unreachable!("senza USCITA la temporanea esiste sempre"),
        };

        let inizio = Instant::now();
        let estratto = zip_handler::extract_series(
            &trovata.archives,
            &destinazione,
            &crate::app_state::no_progress,
        )
        .expect("estrazione della serie");
        let durata = inizio.elapsed();
        let gb = estratto.bytes_written as f64 / 1024.0 / 1024.0 / 1024.0;
        println!("estrazione in {durata:?}");
        println!(
            "  {} file, {} cartelle, {:.2} GB, {:.0} MB/s",
            estratto.files_written,
            estratto.dirs_created,
            gb,
            gb * 1024.0 / durata.as_secs_f64()
        );
        println!(
            "  scartati per sicurezza: {}, collisioni: {}",
            estratto.skipped.len(),
            estratto.collisions.len()
        );
        for voce in estratto.skipped.iter().take(5) {
            println!("    scartato: {voce}");
        }
        for voce in estratto.collisions.iter().take(5) {
            println!("    collisione: {voce}");
        }

        // Un Takeout unito deve avere una radice sola, non una per archivio.
        let radice = estratto.destination.join("Takeout");
        assert!(radice.is_dir(), "manca la radice Takeout nell'albero unito");

        let inizio = Instant::now();
        let sorgente = analyze_folder(&estratto.destination).expect("analisi dell'albero unito");
        println!("analisi delle sezioni in {:?}", inizio.elapsed());
        for sezione in &sorgente.sections {
            println!(
                "  {:<16?} {:>6} file, {:>6.2} GB",
                sezione.section,
                sezione.file_count,
                sezione.total_bytes as f64 / 1024.0 / 1024.0 / 1024.0
            );
        }

        let foto = radice.join("Google Foto");
        if foto.is_dir() {
            let inizio = Instant::now();
            let indice = albums::build_index(&foto, 200).expect("indice degli album");
            println!("album in {:?}", inizio.elapsed());
            println!(
                "  {} album, {} cartelle per anno, {} coppie modificate",
                indice.albums.len(),
                indice.year_folders.len(),
                indice.edited_pairs.len()
            );
            assert!(
                !indice.albums.is_empty() && !indice.year_folders.is_empty(),
                "annate e album vanno distinti entrambi"
            );

            let inizio = Instant::now();
            let scansione = exif_parser::scan_directory(&foto, SAMPLE_SIZE).expect("scansione");
            println!("scansione foto in {:?}", inizio.elapsed());
            println!(
                "  {} media, {:.2} GB, {} con sidecar, {} con coordinate",
                scansione.media_count,
                scansione.total_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
                scansione.with_sidecar,
                scansione.with_geo
            );
            println!(
                "  {} da riparare, {} senza EXIF, {} con data dal nome, {} illeggibili",
                scansione.needs_repair,
                scansione.without_exif,
                scansione.date_from_filename,
                scansione.unreadable_count
            );

            // I sidecar generati da Google esistono per quasi tutti i media:
            // se qui ne risultassero pochi, il riconoscimento del nome
            // troncato a 46 caratteri avrebbe smesso di funzionare.
            assert!(
                scansione.with_sidecar * 10 > scansione.media_count * 8,
                "troppi media senza sidecar: {} su {}",
                scansione.with_sidecar,
                scansione.media_count
            );
        }
        println!();
    }

    /// Ripara una cartella vera e poi mette da parte i sidecar applicati.
    ///
    /// Lavora su una copia della cartella indicata, così l'originale resta
    /// intatto e la misura si può ripetere. Serve a vedere all'opera la catena
    /// intera, riscrittura compresa, su file che non sono stati costruiti dal
    /// test stesso:
    ///
    /// ```bash
    /// CARTELLA="~/Downloads/prova-estratta/Takeout/Google Foto/Foto da 2019" \
    ///   cargo test --release ripara_e_mette_da_parte_i_sidecar -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "richiede una cartella sul disco: si lancia a mano con CARTELLA=..."]
    fn ripara_e_mette_da_parte_i_sidecar() {
        use std::time::Instant;

        let Ok(sorgente) = std::env::var("CARTELLA") else {
            println!("CARTELLA non impostata: niente da riparare.");
            return;
        };
        let sorgente = PathBuf::from(sorgente);

        let temp = TempDir::new("ripara-e-sposta");
        let lavoro = temp.path().join("foto");
        let inizio = Instant::now();
        let copiati = copia_ricorsiva(&sorgente, &lavoro);
        println!(
            "\ncopia di lavoro: {copiati} file in {:?}",
            inizio.elapsed()
        );

        let inizio = Instant::now();
        let riparazione = exif_parser::apply_metadata(
            &lavoro,
            &exif_parser::WriteOptions {
                mode: exif_parser::WriteMode::InPlace,
                ..Default::default()
            },
            &crate::app_state::no_progress,
        )
        .expect("riparazione");
        println!("riparazione in {:?}", inizio.elapsed());
        println!(
            "  {} candidati, {} EXIF scritti, {} date allineate, {} errori",
            riparazione.candidates,
            riparazione.exif_written,
            riparazione.file_times_written,
            riparazione.failures.len()
        );

        let messi_da_parte = temp.path().join("sidecar");
        let inizio = Instant::now();
        let spostamento = drive::sweep_applied_sidecars(
            &lavoro,
            &messi_da_parte,
            20,
            &crate::app_state::no_progress,
        )
        .expect("spostamento dei sidecar");
        println!("spostamento in {:?}", inizio.elapsed());
        println!(
            "  {} spostati ({:.1} kB), {} lasciati",
            spostamento.moved,
            spostamento.bytes_moved as f64 / 1024.0,
            spostamento.kept
        );
        for motivo in &spostamento.kept_reasons {
            println!("    {:>5} per {:?}", motivo.count, motivo.reason);
        }
        assert!(
            spostamento.failures.is_empty(),
            "{:?}",
            spostamento.failures
        );

        // Ciò che è stato riparato non deve restare indietro, e ciò che non è
        // stato riparato non deve essere toccato.
        assert!(
            spostamento.moved > 0,
            "una riparazione riuscita deve liberare qualche sidecar"
        );

        let inizio = Instant::now();
        let ripristino =
            drive::restore_quarantine(&spostamento.manifest.clone().expect("registro scritto"))
                .expect("ripristino");
        println!("ripristino in {:?}", inizio.elapsed());
        assert_eq!(
            ripristino.restored, spostamento.moved,
            "il ripristino deve rimettere tutto ciò che era stato spostato"
        );
        assert!(ripristino.failures.is_empty(), "{:?}", ripristino.failures);
        println!();
    }

    /// Copia una cartella con tutto ciò che contiene, restituendo i file scritti.
    fn copia_ricorsiva(sorgente: &Path, destinazione: &Path) -> usize {
        let mut scritti = 0;
        for entry in walkdir::WalkDir::new(sorgente)
            .follow_links(false)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
        {
            let relativo = entry
                .path()
                .strip_prefix(sorgente)
                .expect("percorso relativo");
            let target = destinazione.join(relativo);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).expect("cartella di destinazione");
            }
            std::fs::copy(entry.path(), &target).expect("copia del file");
            scritti += 1;
        }
        scritti
    }

    #[test]
    fn rifiuta_una_sorgente_non_riconosciuta() {
        let temp = TempDir::new("ignota");
        let file = temp.path().join("note.txt");
        write(&file, "contenuto qualsiasi");

        // Un file che non è un archivio Takeout non deve essere accettato.
        assert!(!zip_handler::is_takeout_archive(&file));

        // Una cartella senza sezioni note produce un avviso, non un errore.
        let summary = analyze_folder(temp.path()).expect("analisi cartella vuota");
        assert!(summary.sections.is_empty());
        assert_eq!(summary.warnings.len(), 1);
    }
}
