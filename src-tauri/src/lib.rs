// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Open Takeout Hub: elaborazione locale degli export Google Takeout.
//!
//! Regola architetturale del progetto: nessun modulo apre connessioni di rete.
//! Non ci sono client HTTP nel grafo delle dipendenze, non c'è telemetria, non
//! c'è updater automatico. I dati letti restano sul disco dell'utente e in
//! memoria per la durata della sessione.

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

use app_state::{
    trace_dev, AppInfo, AppState, ExportReport, LoadedSource, Phase, Preferences, PrivacyReport,
    Progress, Result, SectionSummary, SourceKind, SourceSummary, TakeoutError, TakeoutSection,
};
use calendar::CalendarReport;
use contacts::ContactsReport;
use drive::{CleanOptions, CleanPlan, CleanReport, DriveReport, RestoreReport};
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
            label: section.label().to_string(),
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
        summary.warnings.push(
            "Nessuna sezione Takeout riconosciuta: verifica di aver selezionato la cartella che contiene Google Foto, Drive o Contatti.".to_string(),
        );
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
            label: section.label().to_string(),
            path: path.join(&dir_name),
            dir_name,
            file_count: 0,
            total_bytes: 0,
        });
    }

    if !archive.rejected.is_empty() {
        summary.warnings.push(format!(
            "{} voci dell'archivio hanno percorsi non sicuri e verranno ignorate in estrazione.",
            archive.rejected.len()
        ));
    }
    summary
        .warnings
        .push("Archivio non estratto: estrailo per analizzare foto, contatti e Drive.".to_string());

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
        drive::plan_clean(Path::new(&path), &options, &sink)
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

/// Rimette al loro posto i file spostati in quarantena.
#[tauri::command]
async fn restore_quarantine(manifest: String) -> Result<RestoreReport> {
    in_background(move || drive::restore_quarantine(Path::new(&manifest))).await
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
            restore_quarantine,
            scan_calendar,
            export_contacts,
            export_calendar,
            privacy_report,
            app_info,
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
