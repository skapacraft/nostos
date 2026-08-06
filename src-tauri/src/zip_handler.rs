// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Ispezione ed estrazione degli archivi `takeout-*.zip`.
//!
//! Google consegna il Takeout come serie di ZIP numerati. Qui l'archivio viene
//! letto in streaming: nessuna copia temporanea, nessuna estrazione implicita.
//! L'estrazione avviene solo su richiesta esplicita e verso una destinazione
//! scelta dall'utente.

use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app_state::{Result, TakeoutError};

/// Voce di un archivio, senza il contenuto.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub compressed_size: u64,
}

/// Riepilogo dell'ispezione di un archivio.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSummary {
    pub path: PathBuf,
    pub entry_count: usize,
    pub file_count: usize,
    pub uncompressed_bytes: u64,
    pub compressed_bytes: u64,
    /// Cartelle di primo livello (tipicamente `Takeout/`).
    pub top_level: Vec<String>,
    /// Voci rifiutate perché con percorso non sicuro.
    pub rejected: Vec<String>,
}

/// Esito di un'estrazione.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractReport {
    pub destination: PathBuf,
    /// Archivi effettivamente elaborati, in ordine di numerazione.
    pub archives: Vec<PathBuf>,
    pub files_written: usize,
    pub dirs_created: usize,
    pub bytes_written: u64,
    /// Voci scartate perché con percorso non sicuro.
    pub skipped: Vec<String>,
    /// Percorsi presenti in più di un archivio della serie.
    pub collisions: Vec<String>,
}

/// Serie di archivi che compongono un unico Takeout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSeries {
    /// Prefisso comune, senza il numero progressivo.
    pub prefix: String,
    pub archives: Vec<PathBuf>,
    /// Numeri mancanti nella sequenza, se ce ne sono.
    pub missing: Vec<u32>,
    pub total_compressed_bytes: u64,
}

/// Riconosce un archivio Takeout dal nome del file.
pub fn is_takeout_archive(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip") && (lower.starts_with("takeout") || lower.contains("takeout"))
}

/// Apre l'archivio in sola lettura.
fn open_archive(path: &Path) -> Result<zip::ZipArchive<File>> {
    let file = File::open(path).map_err(|e| TakeoutError::io(path, e))?;
    zip::ZipArchive::new(file).map_err(|e| TakeoutError::Archive(format!("{path:?}: {e}")))
}

/// Elenca il contenuto senza estrarre nulla.
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

/// Restituisce le prime `limit` voci, utile per l'anteprima nella UI.
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

/// Scompone il nome di un archivio in prefisso di serie e numero progressivo.
///
/// Google numera gli export come `takeout-20260805T090000Z-001.zip`. Ogni
/// archivio è autonomo e contiene una fetta dell'albero: non è un archivio
/// spezzato in più volumi, quindi ognuno si apre e si legge da solo.
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

/// Individua tutti gli archivi della stessa serie a partire da uno qualsiasi.
///
/// Se il nome non segue lo schema numerato, la serie contiene solo il file
/// indicato: un Takeout piccolo sta in un archivio solo.
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

    // Un numero mancante di solito significa un download interrotto: va detto
    // prima di estrarre, non dopo aver scoperto che mancano metà delle foto.
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

/// Estrae un singolo archivio in `destination`.
///
/// Ogni percorso viene normalizzato e verificato: le voci assolute, con `..` o
/// con prefissi di volume vengono scartate invece di essere scritte fuori dalla
/// destinazione (zip-slip, CVE-2018-1000544 e simili).
pub fn extract(path: &Path, destination: &Path) -> Result<ExtractReport> {
    extract_series(
        std::slice::from_ref(&path.to_path_buf()),
        destination,
        &crate::app_state::no_progress,
    )
}

/// Estrae un'intera serie di archivi in un unico albero di destinazione.
///
/// Gli archivi vengono uniti: le cartelle si sovrappongono senza conflitto,
/// mentre un file già scritto da un archivio precedente non viene sovrascritto
/// ma registrato come collisione. In un Takeout integro le collisioni sono
/// zero, quindi trovarne una segnala un problema nel download.
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

    // Conteggio preliminare, per dare alla barra di avanzamento un totale vero.
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

            let target = dest_root.join(&relative);
            // Seconda barriera: anche dopo la normalizzazione il target deve
            // restare sotto la radice di destinazione.
            if !target.starts_with(&dest_root) {
                return Err(TakeoutError::UnsafeEntry(raw_name));
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
            // `io::copy` lavora a blocchi: un file da 4 GB non entra in memoria.
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

    Ok(report)
}

/// Normalizza il nome di una voce ZIP in un percorso relativo sicuro.
///
/// Restituisce `None` se la voce tenta di uscire dalla destinazione.
fn safe_relative_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('\0') {
        return None;
    }
    // Gli ZIP usano `/` come separatore anche quando prodotti su Windows, ma
    // alcuni tool scrivono `\`: normalizziamo prima di analizzare.
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
    fn accetta_percorsi_relativi_normali() {
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
    fn rifiuta_i_tentativi_di_zip_slip() {
        assert_eq!(safe_relative_path("../../etc/passwd"), None);
        assert_eq!(safe_relative_path("/etc/passwd"), None);
        assert_eq!(safe_relative_path("Takeout/../../fuori.txt"), None);
        assert_eq!(safe_relative_path("..\\..\\Windows\\System32"), None);
        assert_eq!(safe_relative_path(""), None);
    }

    #[test]
    fn riconosce_i_nomi_degli_archivi_takeout() {
        assert!(is_takeout_archive(Path::new(
            "takeout-20260805T090000Z-001.zip"
        )));
        assert!(is_takeout_archive(Path::new("/tmp/Takeout.zip")));
        assert!(!is_takeout_archive(Path::new("foto.zip")));
        assert!(!is_takeout_archive(Path::new("takeout.tgz")));
    }

    #[test]
    fn scompone_il_nome_in_serie_e_numero() {
        assert_eq!(
            series_key(Path::new("/t/takeout-20260805T090000Z-001.zip")),
            Some(("takeout-20260805T090000Z".to_string(), 1))
        );
        assert_eq!(
            series_key(Path::new("/t/takeout-20260805T090000Z-012.zip")),
            Some(("takeout-20260805T090000Z".to_string(), 12))
        );
        // Senza numero progressivo non c'è serie: è un archivio unico.
        assert_eq!(series_key(Path::new("/t/Takeout.zip")), None);
        assert_eq!(series_key(Path::new("/t/takeout-finale.zip")), None);
    }

    /// Crea un archivio con le voci indicate, per i test di estrazione.
    fn build_archive(path: &Path, entries: &[(&str, &str)]) {
        let file = File::create(path).expect("creazione archivio");
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        for (name, content) in entries {
            if name.ends_with('/') {
                writer.add_directory(*name, options).expect("cartella");
            } else {
                writer.start_file(*name, options).expect("voce");
                std::io::Write::write_all(&mut writer, content.as_bytes()).expect("contenuto");
            }
        }

        writer.finish().expect("chiusura archivio");
    }

    #[test]
    fn unisce_gli_archivi_di_una_serie_in_un_solo_albero() {
        let temp = crate::app_state::testing::TempDir::new("serie");
        let dir = temp.path();

        // Google spezza l'export in archivi autonomi: la cartella `Takeout/` e
        // le sottocartelle di sezione ricompaiono in ognuno.
        build_archive(
            &dir.join("takeout-20260805T090000Z-001.zip"),
            &[
                ("Takeout/", ""),
                ("Takeout/Google Foto/", ""),
                ("Takeout/Google Foto/IMG_0001.JPG", "primo"),
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
        // Un archivio di un altro export non deve entrare nella serie.
        build_archive(
            &dir.join("takeout-20250101T000000Z-001.zip"),
            &[("x.txt", "estraneo")],
        );

        let series = discover_series(&dir.join("takeout-20260805T090000Z-002.zip"))
            .expect("individuazione serie");
        assert_eq!(
            series.archives.len(),
            2,
            "solo gli archivi dello stesso export"
        );
        assert!(series.missing.is_empty());
        assert!(series.archives[0].ends_with("takeout-20260805T090000Z-001.zip"));

        let dest = dir.join("estratto");
        let report = extract_series(&series.archives, &dest, &crate::app_state::no_progress)
            .expect("estrazione della serie");

        assert_eq!(report.files_written, 3);
        assert!(report.skipped.is_empty());
        assert!(
            report.collisions.is_empty(),
            "le cartelle ripetute non sono collisioni"
        );

        assert_eq!(
            std::fs::read_to_string(dest.join("Takeout/Google Foto/IMG_0001.JPG")).unwrap(),
            "primo"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("Takeout/Google Foto/IMG_0002.JPG")).unwrap(),
            "secondo"
        );
        assert!(dest.join("Takeout/Drive/relazione.docx").is_file());
    }

    #[test]
    fn segnala_i_numeri_mancanti_nella_serie() {
        let temp = crate::app_state::testing::TempDir::new("buchi");
        let dir = temp.path();

        for number in ["001", "003"] {
            build_archive(
                &dir.join(format!("takeout-20260805T090000Z-{number}.zip")),
                &[("Takeout/nota.txt", "x")],
            );
        }

        let series = discover_series(&dir.join("takeout-20260805T090000Z-001.zip")).expect("serie");
        assert_eq!(series.archives.len(), 2);
        assert_eq!(series.missing, vec![2], "il download è incompleto");
    }

    #[test]
    fn registra_le_collisioni_senza_sovrascrivere() {
        let temp = crate::app_state::testing::TempDir::new("collisioni");
        let dir = temp.path();

        build_archive(
            &dir.join("takeout-serie-001.zip"),
            &[("Takeout/doppio.txt", "originale")],
        );
        build_archive(
            &dir.join("takeout-serie-002.zip"),
            &[("Takeout/doppio.txt", "sovrascrittura")],
        );

        let series = discover_series(&dir.join("takeout-serie-001.zip")).expect("serie");
        let dest = dir.join("estratto");
        let report = extract_series(&series.archives, &dest, &crate::app_state::no_progress)
            .expect("estrazione");

        assert_eq!(report.files_written, 1);
        assert_eq!(report.collisions.len(), 1);
        // Il primo vince: il secondo non deve poter riscrivere il contenuto.
        assert_eq!(
            std::fs::read_to_string(dest.join("Takeout/doppio.txt")).unwrap(),
            "originale"
        );
    }

    #[test]
    fn riporta_lavanzamento_durante_lestrazione() {
        use crate::app_state::{Phase, Progress};
        use std::sync::Mutex;

        let temp = crate::app_state::testing::TempDir::new("avanzamento");
        let dir = temp.path();
        build_archive(
            &dir.join("takeout-p-001.zip"),
            &[("Takeout/a.txt", "a"), ("Takeout/b.txt", "b")],
        );

        let eventi: Mutex<Vec<Progress>> = Mutex::new(Vec::new());
        let sink = |p: Progress| eventi.lock().unwrap().push(p);

        extract_series(
            &[dir.join("takeout-p-001.zip")],
            &dir.join("estratto"),
            &sink,
        )
        .expect("estrazione");

        let eventi = eventi.into_inner().unwrap();
        assert!(eventi.len() >= 3, "scansione, file, completamento");
        assert_eq!(eventi.first().unwrap().phase, Phase::Scanning);

        let ultimo = eventi.last().unwrap();
        assert_eq!(ultimo.phase, Phase::Done);
        assert_eq!(ultimo.done, ultimo.total);
    }
}
