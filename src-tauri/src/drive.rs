// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Analisi dell'export Google Drive.
//!
//! Due cose sorprendono chi apre un Takeout di Drive per la prima volta:
//!
//! 1. I documenti nativi (Documenti, Fogli, Presentazioni) vengono convertiti
//!    in `.docx` / `.xlsx` / `.pptx` o `.pdf`, non esportati nel formato
//!    originale.
//! 2. I file solo condivisi con l'utente e le scorciatoie compaiono come
//!    segnaposto `.gdoc` / `.gsheet` / `.gslides`: sono JSON di poche centinaia
//!    di byte contenenti un URL. Il contenuto non è nell'archivio.
//!
//! Il secondo punto è la fonte più comune di "backup" incompleti, quindi il
//! modulo lo rileva e lo riporta esplicitamente.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::app_state::{trace_dev, Phase, Progress, ProgressSink, Result, TakeoutError};

/// Estensioni dei segnaposto Google: file senza contenuto reale.
const STUB_EXTENSIONS: &[&str] = &[
    "gdoc", "gsheet", "gslides", "gdraw", "gform", "gsite", "gmap", "gjam", "gtable", "gscript",
    "glink", "gnote",
];

/// File di servizio dei sistemi operativi, senza valore per l'utente.
const JUNK_NAMES: &[&str] = &[
    ".DS_Store",
    "desktop.ini",
    "Thumbs.db",
    "ehthumbs.db",
    ".localized",
    "Icon\r",
];

/// Prefissi dei file di servizio: `._nome` è la parte AppleDouble di un file.
const JUNK_PREFIXES: &[&str] = &["._"];

/// Cartelle di servizio: tutto ciò che sta dentro è spazzatura.
const JUNK_DIRS: &[&str] = &[
    "__MACOSX",
    ".Spotlight-V100",
    ".Trashes",
    ".fseventsd",
    ".TemporaryItems",
];

/// Nome del registro scritto nella quarantena.
const MANIFEST_NAME: &str = "oth-quarantena.json";

/// Dimensione del buffer di lettura durante l'hashing.
///
/// L'hash è calcolato in streaming: un file da 10 GB occupa comunque solo
/// questo buffer, non entra mai in memoria per intero.
const HASH_BUFFER: usize = 64 * 1024;

/// Categoria merceologica di un file.
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
    /// Segnaposto Google privo di contenuto.
    Placeholder,
    Other,
}

impl FileCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Document => "Documenti",
            Self::Spreadsheet => "Fogli di calcolo",
            Self::Presentation => "Presentazioni",
            Self::Pdf => "PDF",
            Self::Image => "Immagini",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Archive => "Archivi",
            Self::Code => "Codice",
            Self::Placeholder => "Segnaposto senza contenuto",
            Self::Other => "Altro",
        }
    }

    /// Deduce la categoria dall'estensione.
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

/// Aggregato per categoria.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryStats {
    pub category: FileCategory,
    pub label: String,
    pub file_count: usize,
    pub total_bytes: u64,
}

/// Segnaposto rilevato, con il riferimento che punta al contenuto online.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceholderFile {
    pub path: PathBuf,
    pub file_name: String,
    pub kind: String,
    /// URL contenuto nel segnaposto, mostrato come testo e mai aperto dall'app.
    pub target_url: Option<String>,
}

/// Gruppo di file con stesso nome e stessa dimensione.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub file_name: String,
    pub size_bytes: u64,
    pub paths: Vec<PathBuf>,
}

/// File più pesanti trovati nell'export.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub path: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
}

/// Esito dell'analisi di una cartella Drive.
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
    /// Byte recuperabili eliminando i duplicati.
    pub duplicate_bytes: u64,
    pub largest_files: Vec<LargeFile>,
    pub warnings: Vec<String>,
}

/// Legge l'URL contenuto in un segnaposto Google.
///
/// Il file è un piccolo JSON: se non è interpretabile restituiamo `None`
/// invece di far fallire l'intera scansione.
fn read_placeholder_url(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Percorre la cartella Drive e produce il report.
///
/// `max_items` limita la lunghezza degli elenchi restituiti alla UI: i
/// conteggi restano completi.
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
                report.warnings.push(err.to_string());
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
                report.warnings.push(format!("{}: {err}", path.display()));
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

        // I duplicati a dimensione zero sono rumore: cartelle vuote esportate,
        // file segnaposto, artefatti di sincronizzazione.
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
            label: category.label().to_string(),
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
            // Ogni copia oltre la prima è spazio recuperabile.
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
        report.warnings.push(format!(
            "{} file sono segnaposto Google senza contenuto: l'export non include i dati, solo un riferimento online.",
            report.placeholder_count
        ));
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Pulizia
// ---------------------------------------------------------------------------

/// Come trattare i file da rimuovere.
///
/// Manca di proposito una modalità che cancelli: un export è spesso l'unica
/// copia rimasta di quei dati, e una deduplica sbagliata su una cancellazione
/// non si annulla. Le due modalità operative producono entrambe qualcosa che si
/// può disfare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanMode {
    /// Calcola il piano senza toccare nulla.
    DryRun,
    /// Costruisce altrove un albero pulito, lasciando intatta l'origine.
    CopyToOutput,
    /// Sposta spazzatura e copie in eccesso in una cartella di quarantena,
    /// scrivendo un registro che permette di rimettere tutto a posto.
    Quarantine,
}

/// Motivo per cui un file è stato spostato in quarantena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuarantineReason {
    Junk,
    Duplicate,
    /// File affiancato che segue il media rimosso.
    Companion,
}

/// Parametri della pulizia.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanOptions {
    pub mode: CleanMode,
    /// Albero di uscita oppure radice della quarantena, secondo la modalità.
    pub destination: Option<PathBuf>,
    pub remove_junk: bool,
    pub remove_duplicates: bool,
    /// Porta con sé i file affiancati quando si rimuove un media.
    ///
    /// Serve su Google Foto: togliere `IMG_1268 2.JPG` e lasciare indietro
    /// `IMG_1268 2.JPG.supplemental-metadata.json` produce un sidecar orfano
    /// che non descrive più nulla.
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

/// Gruppo di file con contenuto identico, verificato per hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentDuplicateGroup {
    /// Hash BLAKE3 del contenuto, abbreviato.
    pub hash: String,
    pub size_bytes: u64,
    /// La copia che viene conservata.
    pub kept: PathBuf,
    /// Le copie in eccesso.
    pub copies: Vec<PathBuf>,
}

/// Piano di pulizia: che cosa succederebbe, senza che sia successo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanPlan {
    pub root: PathBuf,
    pub files_scanned: usize,
    /// File che resterebbero al loro posto.
    pub files_kept: usize,
    pub duplicate_copies: usize,
    pub junk_files: usize,
    /// Sidecar e simili che seguiranno i media rimossi.
    pub companion_files: usize,
    pub reclaimable_bytes: u64,
    /// Byte effettivamente letti per calcolare gli hash.
    pub hashed_bytes: u64,
    pub duplicate_groups: Vec<ContentDuplicateGroup>,
    pub junk_sample: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

/// Voce del registro di quarantena.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineEntry {
    pub original: PathBuf,
    pub quarantined: PathBuf,
    pub reason: QuarantineReason,
    pub size_bytes: u64,
}

/// Registro scritto nella quarantena, che rende reversibile l'operazione.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineManifest {
    pub created_at: DateTime<Utc>,
    pub source_root: PathBuf,
    pub entries: Vec<QuarantineEntry>,
}

/// Esito di una pulizia eseguita.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanReport {
    pub mode: Option<CleanMode>,
    pub destination: Option<PathBuf>,
    pub files_kept: usize,
    pub duplicates_handled: usize,
    pub junk_handled: usize,
    /// Sidecar spostati insieme ai media a cui appartenevano.
    pub companions_handled: usize,
    pub bytes_reclaimed: u64,
    /// Percorso del registro, presente solo in quarantena.
    pub manifest: Option<PathBuf>,
    pub failures: Vec<String>,
}

/// Esito di un ripristino dalla quarantena.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub restored: usize,
    pub skipped_existing: usize,
    pub failures: Vec<String>,
}

/// Vero se il file è un artefatto del sistema operativo.
pub fn is_junk(path: &Path) -> bool {
    // Una cartella di servizio contamina tutto il suo contenuto.
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

/// Nomi dei file contenuti in ogni cartella, raccolti durante la scansione.
///
/// Serve a cercare i file affiancati senza rileggere la cartella ogni volta.
type DirIndex = HashMap<PathBuf, Vec<String>>;

/// Vero se il file è affiancato a un altro presente nella stessa cartella,
/// cioè se il suo nome è il nome completo di quel file più un suffisso.
///
/// È il rovescio di [`find_companions`]: serve a escludere i sidecar dalla
/// deduplica, perché non sono file autonomi.
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

/// Trova i file affiancati a un media, cioè quelli il cui nome è il nome
/// completo del media seguito da un suffisso.
///
/// È la convenzione dei sidecar di Google Foto: `IMG_1268 2.JPG` è accompagnato
/// da `IMG_1268 2.JPG.supplemental-metadata.json`. La regola è volutamente
/// stretta, sul nome completo con estensione, così `IMG_1268.JPG` non cattura
/// per errore i file di `IMG_1268 2.JPG`.
///
/// La ricerca avviene sull'indice già in memoria e non sul filesystem: la
/// versione che rileggeva la cartella a ogni chiamata costava il prodotto tra
/// il numero di duplicati e il numero di file nella loro cartella, e su una
/// libreria di ventimila foto portava il piano di pulizia da un secondo a
/// quasi sette minuti.
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

/// Calcola l'hash BLAKE3 del contenuto, leggendo a blocchi.
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

/// Sceglie quale copia conservare in un gruppo di duplicati.
///
/// Vince il percorso più corto, a parità il primo in ordine alfabetico. Non è
/// arbitrario: le copie generate dai sistemi operativi e da Google aggiungono
/// suffissi (`IMG_1268 2.JPG`, `documento (1).pdf`), quindi il nome più corto è
/// quasi sempre l'originale.
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

/// Costruisce il piano di pulizia senza modificare nulla.
///
/// La deduplica avviene in due fasi: prima si raggruppa per dimensione, che è
/// gratis, poi si calcola l'hash solo dei gruppi con più di un file. Su un
/// export dove quasi tutti i file sono unici questo evita di leggere l'intero
/// contenuto del disco.
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

    // L'indice va completo prima di decidere che cosa è deduplicabile: per
    // riconoscere un sidecar serve sapere se esiste il media a cui appartiene,
    // e quel media può comparire dopo di lui nell'ordine di scansione.
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
                plan.warnings.push(err.to_string());
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        plan.files_scanned += 1;
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

        // L'indice si riempie qui, sfruttando una passata che stiamo già
        // facendo, invece di rileggere le cartelle più avanti.
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

        // Un file affiancato non ha vita propria: appartiene al suo media e lo
        // segue quando viene spostato. Trattarlo come candidato indipendente
        // permetterebbe di rimuoverne uno perché il sidecar di un'altra foto
        // ha per caso lo stesso contenuto, lasciando quella foto senza i suoi
        // metadati. È un errore che non si recupera guardando i file rimasti.
        if options.move_companions && is_companion(path, &dir_index) {
            plan.files_kept += 1;
            continue;
        }

        // I file vuoti hanno tutti lo stesso hash: raggrupparli produrrebbe un
        // gruppo enorme di "duplicati" che non lo sono in alcun senso utile.
        if options.remove_duplicates && size > 0 {
            by_size.entry(size).or_default().push(path.to_path_buf());
        } else {
            plan.files_kept += 1;
        }
    }

    plan.junk_files = junk.len();
    plan.junk_sample = junk.into_iter().take(50).collect();

    // Solo i gruppi con più di un file per dimensione meritano un hash.
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
        "pulizia: {} file esaminati, {} da verificare per contenuto",
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

    // I conteggi restano completi, l'elenco no: è quello che attraversa il
    // canale IPC verso l'interfaccia, e su una libreria vera diventerebbe
    // qualche megabyte di JSON a ogni scansione.
    plan.duplicate_groups.truncate(max_items);

    progress(Progress::new(Phase::Done, to_hash, to_hash, 0));
    Ok(plan)
}

/// Verifica che la destinazione non stia dentro la sorgente.
fn check_destination(root: &Path, destination: &Path) -> Result<()> {
    if destination.starts_with(root) {
        return Err(TakeoutError::Metadata(
            "la destinazione non può stare dentro la cartella di origine".to_string(),
        ));
    }
    Ok(())
}

/// Esegue la pulizia secondo le opzioni indicate.
pub fn clean(
    root: &Path,
    options: &CleanOptions,
    progress: ProgressSink<'_>,
) -> Result<CleanReport> {
    // Qui l'elenco dei duplicati serve intero, non troncato per la UI.
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

    let destination = options.destination.as_ref().ok_or_else(|| {
        TakeoutError::Metadata("questa modalità richiede una destinazione".to_string())
    })?;
    check_destination(root, destination)?;
    std::fs::create_dir_all(destination).map_err(|e| TakeoutError::io(destination, e))?;

    // L'insieme dei file da rimuovere: le copie in eccesso e la spazzatura.
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
    // `junk_sample` è troncato per la UI: qui serve l'elenco completo.
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
                // `rename` fallisce tra volumi diversi: in quel caso si copia e
                // si rimuove, che è l'unico modo di spostare tra filesystem.
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

            // Il registro va scritto anche se qualche spostamento è fallito:
            // senza, ciò che è stato spostato non si recupera più.
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
        "pulizia conclusa: {} duplicati, {} spazzatura, {} byte, {} errori",
        report.duplicates_handled,
        report.junk_handled,
        report.bytes_reclaimed,
        report.failures.len()
    );

    Ok(report)
}

/// Rimette al loro posto i file spostati in quarantena.
///
/// È la funzione che rende vera la parola "reversibile": senza, la quarantena
/// sarebbe solo una cancellazione con un nome più gentile.
pub fn restore_quarantine(manifest_path: &Path) -> Result<RestoreReport> {
    let content =
        std::fs::read_to_string(manifest_path).map_err(|e| TakeoutError::io(manifest_path, e))?;
    let manifest: QuarantineManifest = serde_json::from_str(&content)
        .map_err(|e| TakeoutError::Metadata(format!("{}: {e}", manifest_path.display())))?;

    let mut report = RestoreReport::default();

    for entry in &manifest.entries {
        // Se nel frattempo qualcosa è ricomparso all'origine, non lo si
        // sovrascrive: meglio lasciare il file in quarantena e dirlo.
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
        "ripristino: {} rimessi a posto, {} saltati, {} errori",
        report.restored,
        report.skipped_existing,
        report.failures.len()
    );

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifica_per_estensione() {
        assert_eq!(
            FileCategory::from_path(Path::new("relazione.docx")),
            FileCategory::Document
        );
        assert_eq!(
            FileCategory::from_path(Path::new("bilancio.xlsx")),
            FileCategory::Spreadsheet
        );
        assert_eq!(
            FileCategory::from_path(Path::new("foto.HEIC")),
            FileCategory::Image
        );
        assert_eq!(
            FileCategory::from_path(Path::new("senza_estensione")),
            FileCategory::Other
        );
    }

    use crate::app_state::no_progress;
    use crate::app_state::testing::{write_file, TempDir};

    /// Albero di prova con duplicati veri, sosia per dimensione e spazzatura.
    fn build_drive(root: &Path) -> PathBuf {
        let drive = root.join("Drive");

        // Stesso contenuto, nomi e cartelle diversi: duplicati autentici.
        write_file(&drive.join("relazione.docx"), "contenuto A");
        write_file(&drive.join("copia").join("relazione.docx"), "contenuto A");

        // Stessa dimensione dei precedenti ma contenuto diverso: la deduplica
        // per nome e dimensione li sbaglierebbe, quella per contenuto no.
        write_file(&drive.join("altro.txt"), "contenuto B");
        write_file(&drive.join("terzo.txt"), "contenuto C");

        // Spazzatura di sistema.
        write_file(&drive.join(".DS_Store"), "spazzatura");
        write_file(&drive.join("sub").join("._nascosto"), "appledouble");
        write_file(&drive.join("__MACOSX").join("roba.txt"), "spazzatura");

        drive
    }

    #[test]
    fn riconosce_la_spazzatura_di_sistema() {
        assert!(is_junk(Path::new("/x/.DS_Store")));
        assert!(is_junk(Path::new("/x/desktop.ini")));
        assert!(is_junk(Path::new("/x/Thumbs.db")));
        assert!(is_junk(Path::new("/x/._foto.jpg")));
        assert!(is_junk(Path::new("/x/__MACOSX/qualsiasi.txt")));
        // I file veri non devono essere toccati.
        assert!(!is_junk(Path::new("/x/relazione.docx")));
        assert!(!is_junk(Path::new("/x/.gitignore")));
    }

    #[test]
    fn conserva_la_copia_dal_nome_piu_corto() {
        let mut paths = vec![
            PathBuf::from("/foto/IMG_1268 2.JPG"),
            PathBuf::from("/foto/IMG_1268.JPG"),
        ];
        // Il suffisso " 2" identifica la copia, non l'originale.
        assert_eq!(choose_kept(&mut paths), PathBuf::from("/foto/IMG_1268.JPG"));
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn distingue_i_duplicati_veri_dai_sosia_per_dimensione() {
        let temp = TempDir::new("drive-piano");
        let drive = build_drive(temp.path());

        let plan =
            plan_clean(&drive, &CleanOptions::default(), usize::MAX, &no_progress).expect("piano");

        assert_eq!(plan.files_scanned, 7);
        assert_eq!(plan.junk_files, 3, "DS_Store, AppleDouble e __MACOSX");
        assert_eq!(
            plan.duplicate_groups.len(),
            1,
            "solo relazione.docx è duplicato davvero"
        );
        assert_eq!(plan.duplicate_copies, 1);

        // I due file di uguale dimensione ma contenuto diverso restano.
        let gruppo = &plan.duplicate_groups[0];
        assert!(gruppo.kept.ends_with("relazione.docx"));
        assert_eq!(gruppo.copies.len(), 1);
        assert!(gruppo.copies[0].ends_with("copia/relazione.docx"));

        // Il piano non ha toccato nulla.
        assert!(drive.join("copia").join("relazione.docx").is_file());
        assert!(drive.join(".DS_Store").is_file());
    }

    #[test]
    fn la_simulazione_non_scrive_nulla() {
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
        assert_eq!(prima, dopo, "la simulazione deve lasciare l'albero intatto");
    }

    #[test]
    fn la_quarantena_si_annulla_completamente() {
        let temp = TempDir::new("drive-quarantena");
        let drive = build_drive(temp.path());
        let quarantena = temp.path().join("quarantena");

        /// Istantanea di percorsi e contenuti, per confrontare prima e dopo.
        fn istantanea(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
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

        let prima = istantanea(&drive);
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

        // I file sono stati spostati, non cancellati.
        assert!(!drive.join("copia").join("relazione.docx").exists());
        assert!(!drive.join(".DS_Store").exists());
        assert_eq!(istantanea(&drive).len(), 3, "restano i tre file unici");
        assert!(quarantena.join("copia").join("relazione.docx").is_file());

        // Il registro esiste ed è leggibile.
        let manifest = report.manifest.expect("registro scritto");
        assert!(manifest.is_file());

        // E ora la parte che conta: si torna esattamente al punto di partenza.
        let restore = restore_quarantine(&manifest).expect("ripristino");
        assert_eq!(restore.restored, 4);
        assert!(restore.failures.is_empty(), "{:?}", restore.failures);
        assert_eq!(
            istantanea(&drive),
            prima,
            "dopo il ripristino l'albero deve essere identico all'originale"
        );
    }

    #[test]
    fn lalbero_pulito_esclude_duplicati_e_spazzatura() {
        let temp = TempDir::new("drive-copia");
        let drive = build_drive(temp.path());
        let uscita = temp.path().join("pulito");

        let report = clean(
            &drive,
            &CleanOptions {
                mode: CleanMode::CopyToOutput,
                destination: Some(uscita.clone()),
                ..Default::default()
            },
            &no_progress,
        )
        .expect("albero pulito");

        assert!(report.failures.is_empty(), "{:?}", report.failures);

        let prodotti: Vec<String> = WalkDir::new(&uscita)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                e.path()
                    .strip_prefix(&uscita)
                    .unwrap_or(e.path())
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();

        assert_eq!(
            prodotti.len(),
            3,
            "un solo esemplare per contenuto: {prodotti:?}"
        );
        assert!(prodotti.iter().any(|p| p == "relazione.docx"));
        assert!(prodotti.iter().any(|p| p == "altro.txt"));
        assert!(prodotti.iter().any(|p| p == "terzo.txt"));
        assert!(!prodotti.iter().any(|p| p.contains("DS_Store")));
        assert!(!prodotti.iter().any(|p| p.contains("__MACOSX")));

        // L'origine non è stata toccata.
        assert!(drive.join("copia").join("relazione.docx").is_file());
        assert!(drive.join(".DS_Store").is_file());
    }

    #[test]
    fn il_sidecar_segue_il_media_rimosso() {
        let temp = TempDir::new("drive-sidecar");
        let foto = temp.path().join("Google Foto");

        // Due scatti identici come contenuto, come li produce Google quando la
        // stessa foto sta in più album, ognuno con il proprio sidecar.
        write_file(&foto.join("IMG_1268.JPG"), "pixel identici");
        write_file(
            &foto.join("IMG_1268.JPG.supplemental-metadata.json"),
            r#"{"title": "IMG_1268.JPG"}"#,
        );
        write_file(&foto.join("IMG_1268 2.JPG"), "pixel identici");
        write_file(
            &foto.join("IMG_1268 2.JPG.supplemental-metadata.json"),
            r#"{"title": "IMG_1268 2.JPG"}"#,
        );

        let quarantena = temp.path().join("quarantena");
        let report = clean(
            &foto,
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
        assert_eq!(report.companions_handled, 1, "il sidecar deve seguire");

        // Sopravvive l'originale, con il suo sidecar.
        assert!(foto.join("IMG_1268.JPG").is_file());
        assert!(foto
            .join("IMG_1268.JPG.supplemental-metadata.json")
            .is_file());

        // La copia se n'è andata insieme al proprio sidecar: niente orfani.
        assert!(!foto.join("IMG_1268 2.JPG").exists());
        assert!(!foto
            .join("IMG_1268 2.JPG.supplemental-metadata.json")
            .exists());

        // E si torna indietro completamente.
        let manifest = report.manifest.expect("registro");
        let restore = restore_quarantine(&manifest).expect("ripristino");
        assert_eq!(restore.restored, 2);
        assert!(foto.join("IMG_1268 2.JPG").is_file());
        assert!(foto
            .join("IMG_1268 2.JPG.supplemental-metadata.json")
            .is_file());
    }

    /// Due sidecar possono avere contenuto identico anche appartenendo a foto
    /// diverse. Trattarli come duplicati indipendenti ne rimuoverebbe uno, e la
    /// foto rimasta senza perderebbe data e coordinate: un danno che, guardando
    /// i file superstiti, non si vede nemmeno.
    #[test]
    fn i_sidecar_non_vengono_deduplicati_tra_loro() {
        let temp = TempDir::new("sidecar-dedup");
        let foto = temp.path().join("Google Foto");

        // Due foto diverse, con sidecar dal contenuto identico.
        write_file(&foto.join("IMG_1.JPG"), "pixel della prima");
        write_file(&foto.join("IMG_2.JPG"), "pixel della second");
        write_file(&foto.join("IMG_1.JPG.json"), r#"{"t": "1577880000"}"#);
        write_file(&foto.join("IMG_2.JPG.json"), r#"{"t": "1577880000"}"#);

        let plan =
            plan_clean(&foto, &CleanOptions::default(), usize::MAX, &no_progress).expect("piano");

        assert_eq!(
            plan.duplicate_copies, 0,
            "i sidecar identici non sono duplicati da rimuovere"
        );
        assert_eq!(plan.files_scanned, 4);
        assert_eq!(plan.files_kept, 4, "resta tutto");
    }

    #[test]
    fn il_prefisso_dei_companion_non_confonde_nomi_simili() {
        let temp = TempDir::new("drive-prefisso");
        let foto = temp.path().join("f");
        write_file(&foto.join("IMG_1268.JPG"), "a");
        write_file(&foto.join("IMG_1268.JPG.json"), "sidecar del primo");
        write_file(&foto.join("IMG_1268 2.JPG"), "b");
        write_file(&foto.join("IMG_1268 2.JPG.json"), "sidecar del secondo");

        // L'indice è quello che `plan_clean` costruisce durante la scansione.
        let mut indice: DirIndex = HashMap::new();
        indice.insert(
            foto.clone(),
            std::fs::read_dir(&foto)
                .expect("lettura cartella")
                .flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .collect(),
        );

        // `IMG_1268.JPG` non deve rivendicare i file di `IMG_1268 2.JPG`.
        let compagni = find_companions(&foto.join("IMG_1268.JPG"), &indice);
        assert_eq!(compagni.len(), 1);
        assert!(compagni[0].ends_with("IMG_1268.JPG.json"));
    }

    #[test]
    fn rifiuta_una_destinazione_dentro_la_sorgente() {
        let temp = TempDir::new("drive-ricorsione");
        let drive = build_drive(temp.path());

        let esito = clean(
            &drive,
            &CleanOptions {
                mode: CleanMode::Quarantine,
                destination: Some(drive.join("quarantena")),
                ..Default::default()
            },
            &no_progress,
        );

        assert!(esito.is_err(), "una destinazione annidata va rifiutata");
    }

    #[test]
    fn riconosce_i_segnaposto_google() {
        for ext in ["gdoc", "gsheet", "gslides", "gform"] {
            let path = PathBuf::from(format!("appunti.{ext}"));
            assert_eq!(
                FileCategory::from_path(&path),
                FileCategory::Placeholder,
                "{ext} deve essere un segnaposto"
            );
        }
    }
}
