// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Struttura delle cartelle di un export Google Foto.
//!
//! Google non esporta gli album come metadato: li esporta come **cartelle che
//! contengono una seconda copia della foto**. La stessa immagine finisce in
//! `Photos from 2020/` e in `Vacanze in Sicilia/`, byte per byte identica.
//!
//! Questo ha una conseguenza che rende il modulo necessario: una deduplica per
//! contenuto, presa da sola, rimuove le copie negli album e con esse
//! l'unica traccia rimasta dell'appartenenza. I file si recuperano dalla
//! quarantena, l'informazione no. Qui l'appartenenza viene letta e scritta in
//! un manifest prima che qualcuno tocchi i file.
//!
//! Il modulo riconosce anche le versioni modificate (`IMG_1234-edited.jpg`),
//! che Google affianca all'originale e che non sono duplicati: hanno pixel
//! diversi e vanno tenute entrambe.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

use crate::app_state::{ExportReport, Result, TakeoutError};

/// Suffissi che Google aggiunge alle versioni modificate di una foto.
///
/// L'elenco è per lingua dell'account, non per lingua del sistema, e non è
/// documentato da nessuna parte: è materiale raccolto dall'uso reale. Va
/// confrontato sempre in minuscolo e in forma normalizzata NFC.
const EDITED_SUFFIXES: &[&str] = &[
    "-edited",
    "-effects",
    "-smile",
    "-mix",
    "-modificato",
    "-bearbeitet",
    "-bewerkt",
    "-edytowane",
    "-modifié",
    "-ha editado",
    "-editat",
    "-編集済み",
];

/// Nomi di cartelle speciali, che non sono album dell'utente.
const SPECIAL_FOLDERS: &[&str] = &[
    "archive",
    "archivio",
    "archiv",
    "trash",
    "bin",
    "cestino",
    "papelera",
    "corbeille",
    "papierkorb",
    "failed videos",
    "video non riusciti",
];

/// Natura di una cartella dentro l'export di Google Foto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum FolderKind {
    /// Cartella per anno: `Photos from 2020`, `Foto da 2026`.
    Year(i32),
    /// Album creato dall'utente.
    Album,
    /// Archivio, cestino e simili.
    Special,
}

/// Un album, con i file che vi appartengono.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub name: String,
    pub path: PathBuf,
    /// Quanti file contiene, sempre completo.
    pub file_count: usize,
    /// Campione dei nomi contenuti, troncato per la UI.
    pub files: Vec<String>,
}

/// Una foto presente sia in una cartella per anno sia in uno o più album.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumMembership {
    pub file_name: String,
    /// Copia nella cartella per anno, quella da conservare.
    pub canonical: Option<PathBuf>,
    /// Album in cui la stessa foto compare.
    pub albums: Vec<String>,
}

/// Versione modificata affiancata al suo originale.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditedPair {
    pub edited: PathBuf,
    /// Originale corrispondente, se presente nella stessa cartella.
    pub original: Option<PathBuf>,
    /// Suffisso riconosciuto, utile per capire la lingua dell'account.
    pub suffix: String,
}

/// Fotografia della struttura di un export Google Foto.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumIndex {
    pub root: PathBuf,
    pub year_folders: Vec<String>,
    pub albums: Vec<Album>,
    pub special_folders: Vec<String>,
    /// Quante foto compaiono anche in almeno un album, conteggio completo.
    pub membership_count: usize,
    /// Campione delle appartenenze, troncato per la UI.
    pub memberships: Vec<AlbumMembership>,
    /// Quante versioni modificate esistono, conteggio completo.
    pub edited_count: usize,
    pub edited_pairs: Vec<EditedPair>,
    /// Foto presenti solo in un album e in nessuna cartella per anno.
    pub album_only: usize,
    pub warnings: Vec<String>,
}

/// Manifest scritto su disco prima di deduplicare.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumManifest {
    pub created_at: DateTime<Utc>,
    pub source_root: PathBuf,
    pub note: String,
    pub albums: Vec<Album>,
    pub memberships: Vec<AlbumMembership>,
}

/// Riduce una stringa a una forma confrontabile.
///
/// Su macOS i nomi dei file sono in NFD, quindi `-modifié` arriva come `e` più
/// un accento combinante e non corrisponde alla costante scritta in NFC. Senza
/// questa normalizzazione il riconoscimento funzionerebbe in inglese e
/// fallirebbe in francese e giapponese.
fn normalize(value: &str) -> String {
    value.nfc().collect::<String>().to_lowercase()
}

/// Riconosce la natura di una cartella dal suo nome.
///
/// Le cartelle per anno sono localizzate nella lingua dell'account
/// (`Photos from 2020`, `Foto da 2026`), quindi non si possono elencare: si
/// riconoscono dal fatto che terminano con un anno plausibile.
pub fn classify_folder(name: &str) -> FolderKind {
    let normalized = normalize(name.trim());

    if SPECIAL_FOLDERS.iter().any(|s| normalized == *s) {
        return FolderKind::Special;
    }

    if let Some(year) = trailing_year(&normalized) {
        return FolderKind::Year(year);
    }

    FolderKind::Album
}

/// Estrae l'anno finale di un nome di cartella, se plausibile.
fn trailing_year(name: &str) -> Option<i32> {
    let digits: String = name
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    if digits.len() != 4 {
        return None;
    }
    let year: i32 = digits.parse().ok()?;
    // Fuori da questo intervallo è un numero nel nome di un album, non un anno.
    (1900..=2100).contains(&year).then_some(year)
}

/// Se il nome è una versione modificata, restituisce il nome dell'originale e
/// il suffisso riconosciuto.
///
/// `IMG_1234-edited.jpg` diventa `("IMG_1234.jpg", "-edited")`.
pub fn strip_edited_suffix(file_name: &str) -> Option<(String, String)> {
    let path = Path::new(file_name);
    let stem = path.file_stem()?.to_str()?;
    let extension = path.extension().and_then(|e| e.to_str());
    let normalized_stem = normalize(stem);

    let stem_chars: Vec<char> = stem.chars().collect();

    for suffix in EDITED_SUFFIXES {
        let normalized_suffix = normalize(suffix);
        if !normalized_stem.ends_with(&normalized_suffix) {
            continue;
        }

        // Il punto di taglio va cercato sulla stringa originale, non dedotto
        // dalla lunghezza di quella normalizzata: in NFD `é` occupa due
        // caratteri invece di uno, quindi contare sulla forma NFC taglierebbe
        // troppo poco e lascerebbe pezzi di suffisso attaccati al nome.
        // La coda non può essere molto più lunga del suffisso in NFC, così il
        // confronto resta limitato agli ultimi caratteri.
        let max_tail = normalized_suffix.chars().count() * 4;
        let lower = stem_chars.len().saturating_sub(max_tail);

        for cut in (lower..stem_chars.len()).rev() {
            let tail: String = stem_chars[cut..].iter().collect();
            if normalize(&tail) != normalized_suffix {
                continue;
            }
            let base: String = stem_chars[..cut].iter().collect();
            if base.is_empty() {
                break;
            }
            let original = match extension {
                Some(ext) => format!("{base}.{ext}"),
                None => base,
            };
            return Some((original, (*suffix).to_string()));
        }
    }

    None
}

/// Percorre un export Google Foto e ne ricostruisce la struttura.
pub fn build_index(root: &Path, max_items: usize) -> Result<AlbumIndex> {
    crate::app_state::require_existing(root)?;

    let mut index = AlbumIndex {
        root: root.to_path_buf(),
        ..Default::default()
    };

    // Nome file -> cartelle per anno in cui compare.
    let mut in_years: HashMap<String, PathBuf> = HashMap::new();
    // Nome file -> album in cui compare.
    let mut in_albums: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let entries = std::fs::read_dir(root).map_err(|e| TakeoutError::io(root, e))?;

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

        let kind = classify_folder(&name);
        let media: Vec<String> = WalkDir::new(&dir)
            .follow_links(false)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter(|e| crate::exif_parser::is_media_file(e.path()))
            .filter_map(|e| {
                e.path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
            })
            .collect();

        match kind {
            FolderKind::Year(_) => {
                index.year_folders.push(name.clone());
                for file in &media {
                    in_years.insert(file.clone(), dir.join(file));
                }
            }
            FolderKind::Special => index.special_folders.push(name.clone()),
            FolderKind::Album => {
                for file in &media {
                    in_albums
                        .entry(file.clone())
                        .or_default()
                        .push(name.clone());
                }
                index.albums.push(Album {
                    name: name.clone(),
                    path: dir.clone(),
                    file_count: media.len(),
                    files: media,
                });
            }
        }
    }

    for (file_name, albums) in in_albums {
        let canonical = in_years.get(&file_name).cloned();
        if canonical.is_none() {
            index.album_only += 1;
        }
        index.memberships.push(AlbumMembership {
            file_name,
            canonical,
            albums,
        });
    }

    index.year_folders.sort();
    index.special_folders.sort();
    index.albums.sort_by(|a, b| a.name.cmp(&b.name));

    // Le versioni modificate si cercano ovunque: stanno accanto all'originale.
    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() || !crate::exif_parser::is_media_file(entry.path()) {
            continue;
        }
        let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some((original, suffix)) = strip_edited_suffix(name) {
            let candidate = entry.path().with_file_name(&original);
            index.edited_pairs.push(EditedPair {
                edited: entry.path().to_path_buf(),
                original: candidate.is_file().then_some(candidate),
                suffix,
            });
        }
    }

    // I conteggi restano completi, gli elenchi no: attraversano il canale IPC
    // a ogni scansione, e su una libreria vera l'elenco delle appartenenze da
    // solo vale qualche megabyte di JSON. Il manifest completo si ottiene
    // esportandolo, che è l'operazione fatta apposta.
    //
    // Il taglio va in fondo, dopo che ogni elenco è stato riempito: messo
    // prima, lasciava passare intatte le versioni modificate, che vengono
    // raccolte più avanti.
    index.membership_count = index.memberships.len();
    index.edited_count = index.edited_pairs.len();
    index.memberships.truncate(max_items);
    index.edited_pairs.truncate(max_items);
    for album in &mut index.albums {
        album.file_count = album.files.len();
        album.files.truncate(max_items);
    }

    if index.album_only > 0 {
        index.warnings.push(format!(
            "{} foto compaiono solo dentro un album e in nessuna cartella per anno: rimuoverle dagli album le farebbe sparire del tutto.",
            index.album_only
        ));
    }
    if index.membership_count > 0 {
        index.warnings.push(format!(
            "{} foto sono duplicate tra cartelle per anno e album. Esporta il manifest prima di deduplicare, altrimenti l'appartenenza agli album va persa.",
            index.membership_count
        ));
    }

    Ok(index)
}

/// Scrive il manifest degli album.
pub fn export_manifest(root: &Path, destination: &Path) -> Result<ExportReport> {
    // Il manifest deve essere completo: è il documento che conserva il dato,
    // non l'anteprima mostrata nell'interfaccia.
    let index = build_index(root, usize::MAX)?;

    let manifest = AlbumManifest {
        created_at: Utc::now(),
        source_root: index.root.clone(),
        note: "Google Foto esporta gli album come cartelle contenenti una copia \
               della foto. Questo file conserva l'appartenenza agli album prima \
               che le copie vengano deduplicate."
            .to_string(),
        albums: index.albums,
        memberships: index.memberships,
    };

    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| TakeoutError::Metadata(e.to_string()))?;

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TakeoutError::io(parent, e))?;
    }
    std::fs::write(destination, &json).map_err(|e| TakeoutError::io(destination, e))?;

    Ok(ExportReport {
        path: destination.to_path_buf(),
        written: manifest.memberships.len(),
        bytes: json.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::testing::{write_file, TempDir, MINIMAL_JPEG};

    #[test]
    fn riconosce_le_cartelle_per_anno_in_piu_lingue() {
        assert_eq!(classify_folder("Photos from 2020"), FolderKind::Year(2020));
        assert_eq!(classify_folder("Foto da 2026"), FolderKind::Year(2026));
        assert_eq!(classify_folder("2019"), FolderKind::Year(2019));
        // Un numero che non è un anno non deve ingannare.
        assert_eq!(classify_folder("Corsa dei 1000"), FolderKind::Album);
        assert_eq!(classify_folder("Vacanze in Sicilia"), FolderKind::Album);
    }

    #[test]
    fn riconosce_le_cartelle_speciali() {
        assert_eq!(classify_folder("Archive"), FolderKind::Special);
        assert_eq!(classify_folder("Cestino"), FolderKind::Special);
        assert_eq!(classify_folder("TRASH"), FolderKind::Special);
    }

    #[test]
    fn riconosce_le_versioni_modificate() {
        assert_eq!(
            strip_edited_suffix("IMG_1234-edited.jpg"),
            Some(("IMG_1234.jpg".to_string(), "-edited".to_string()))
        );
        assert_eq!(
            strip_edited_suffix("foto-modificato.HEIC"),
            Some(("foto.HEIC".to_string(), "-modificato".to_string()))
        );
        assert_eq!(strip_edited_suffix("IMG_1234.jpg"), None);
        // Un file composto solo dal suffisso non ha un originale sensato.
        assert_eq!(strip_edited_suffix("-edited.jpg"), None);
    }

    /// macOS conserva i nomi in NFD: senza normalizzazione il confronto con la
    /// costante scritta in NFC fallirebbe, e il riconoscimento funzionerebbe
    /// in inglese ma non in francese.
    #[test]
    fn riconosce_i_suffissi_accentati_anche_in_forma_nfd() {
        let nfd: String = "IMG_1-modifie\u{0301}.jpg".to_string();
        assert_ne!(
            nfd, "IMG_1-modifié.jpg",
            "le due forme differiscono in byte"
        );
        assert_eq!(
            strip_edited_suffix(&nfd).map(|(base, _)| base),
            Some("IMG_1.jpg".to_string())
        );
        assert_eq!(
            strip_edited_suffix("IMG_1-modifié.jpg").map(|(base, _)| base),
            Some("IMG_1.jpg".to_string())
        );
    }

    /// Il caso che rende necessario questo modulo: la stessa foto in una
    /// cartella per anno e in un album.
    #[test]
    fn registra_lappartenenza_agli_album() {
        let temp = TempDir::new("album-index");
        let foto = temp.path().join("Google Foto");

        crate::app_state::testing::write_bytes(
            &foto.join("Foto da 2026").join("IMG_1.JPG"),
            MINIMAL_JPEG,
        );
        crate::app_state::testing::write_bytes(
            &foto.join("Vacanze in Sicilia").join("IMG_1.JPG"),
            MINIMAL_JPEG,
        );
        crate::app_state::testing::write_bytes(
            &foto.join("Vacanze in Sicilia").join("IMG_2.JPG"),
            MINIMAL_JPEG,
        );
        write_file(&foto.join("Cestino").join("nota.txt"), "x");

        let index = build_index(&foto, usize::MAX).expect("indice");

        assert_eq!(index.year_folders, vec!["Foto da 2026"]);
        assert_eq!(index.albums.len(), 1);
        assert_eq!(index.albums[0].name, "Vacanze in Sicilia");
        assert_eq!(index.special_folders, vec!["Cestino"]);

        // IMG_1 sta in entrambe: l'appartenenza va registrata.
        let membership = index
            .memberships
            .iter()
            .find(|m| m.file_name == "IMG_1.JPG")
            .expect("appartenenza registrata");
        assert_eq!(membership.albums, vec!["Vacanze in Sicilia"]);
        assert!(membership.canonical.is_some());

        // IMG_2 sta solo nell'album: toglierla la farebbe sparire.
        let solo = index
            .memberships
            .iter()
            .find(|m| m.file_name == "IMG_2.JPG")
            .expect("presente");
        assert!(solo.canonical.is_none());
        assert_eq!(index.album_only, 1);
        assert_eq!(index.warnings.len(), 2);
    }

    #[test]
    fn il_manifest_e_rileggibile() {
        let temp = TempDir::new("album-manifest");
        let foto = temp.path().join("Google Foto");
        crate::app_state::testing::write_bytes(
            &foto.join("Photos from 2020").join("IMG_1.JPG"),
            MINIMAL_JPEG,
        );
        crate::app_state::testing::write_bytes(
            &foto.join("Compleanno").join("IMG_1.JPG"),
            MINIMAL_JPEG,
        );

        let destination = temp.path().join("uscita").join("album.json");
        let report = export_manifest(&foto, &destination).expect("manifest");
        assert_eq!(report.written, 1);

        let content = std::fs::read_to_string(&destination).expect("lettura");
        let manifest: AlbumManifest = serde_json::from_str(&content).expect("rilettura");
        assert_eq!(manifest.memberships[0].albums, vec!["Compleanno"]);
        assert_eq!(manifest.albums[0].name, "Compleanno");
    }
}
