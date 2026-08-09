// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The folder structure of a Google Photos export.
//!
//! Google does not export albums as metadata: it exports them as **folders
//! containing a second copy of the photo**. The same image ends up in
//! `Photos from 2020/` and in `Holidays in Sicily/`, identical byte for byte.
//!
//! That has a consequence which makes this module necessary: deduplication by
//! content, taken on its own, removes the copies inside albums and with them the
//! only remaining trace of membership. The files can be recovered from
//! quarantine, the information cannot. Here membership is read and written to a
//! manifest before anyone touches the files.
//!
//! The module also recognises edited versions (`IMG_1234-edited.jpg`), which
//! Google places beside the original and which are not duplicates: they have
//! different pixels and both are worth keeping.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

use crate::app_state::{ExportReport, Notice, Result, TakeoutError};

/// Suffixes Google appends to the edited version of a photo.
///
/// The list follows the account language, not the system language, and is
/// documented nowhere: it is material gathered from real use. It must always be
/// compared in lowercase and in NFC normalised form.
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

/// Names of special folders, which are not user albums.
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

/// What a folder inside a Google Photos export actually is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum FolderKind {
    /// Year folder: `Photos from 2020`, `Foto da 2026`.
    Year(i32),
    /// Album created by the user.
    Album,
    /// Archive, trash and the like.
    Special,
}

impl FolderKind {
    pub fn is_year(&self) -> bool {
        matches!(self, FolderKind::Year(_))
    }
}

/// An album, with the files belonging to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub name: String,
    pub path: PathBuf,
    /// How many files it holds, always the complete count.
    pub file_count: usize,
    /// Sample of the names inside, truncated for the UI.
    pub files: Vec<String>,
}

/// A photo present both in a year folder and in one or more albums.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumMembership {
    pub file_name: String,
    /// The copy in the year folder, the one to keep.
    pub canonical: Option<PathBuf>,
    /// Albums in which the same photo appears.
    pub albums: Vec<String>,
}

/// An edited version sitting beside its original.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditedPair {
    pub edited: PathBuf,
    /// The matching original, if present in the same folder.
    pub original: Option<PathBuf>,
    /// The suffix recognised, useful for telling the account language.
    pub suffix: String,
}

/// A snapshot of the structure of a Google Photos export.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumIndex {
    pub root: PathBuf,
    pub year_folders: Vec<String>,
    pub albums: Vec<Album>,
    pub special_folders: Vec<String>,
    /// How many photos also appear in at least one album, complete count.
    pub membership_count: usize,
    /// Sample of the memberships, truncated for the UI.
    pub memberships: Vec<AlbumMembership>,
    /// How many edited versions exist, complete count.
    pub edited_count: usize,
    pub edited_pairs: Vec<EditedPair>,
    /// Photos present only in an album and in no year folder.
    pub album_only: usize,
    pub warnings: Vec<Notice>,
}

/// The manifest written to disk before deduplicating.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumManifest {
    pub created_at: DateTime<Utc>,
    pub source_root: PathBuf,
    pub note: String,
    pub albums: Vec<Album>,
    pub memberships: Vec<AlbumMembership>,
}

/// Reduces a string to a comparable form.
///
/// On macOS filenames are in NFD, so `-modifié` arrives as `e` plus a combining
/// accent and does not match the constant written in NFC. Without this
/// normalisation the recognition would work in English and fail in French and
/// Japanese.
fn normalize(value: &str) -> String {
    value.nfc().collect::<String>().to_lowercase()
}

/// Recognises what a folder is from its name.
///
/// Year folders are localised into the account language (`Photos from 2020`,
/// `Foto da 2026`), so they cannot be listed: they are recognised by the fact
/// that they end with a plausible year.
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

/// Like `classify_folder`, but knowing what this export calls its years.
///
/// Ending with a year is not enough to tell `Foto da 2024` from an album called
/// `Christmas 2024`, and getting it wrong is not harmless: an album mistaken for
/// a year never reaches the manifest, which loses precisely the information the
/// manifest exists to save. With the export's own prefix in hand the distinction
/// becomes clear-cut.
pub fn classify_folder_in(name: &str, year_prefix: Option<&str>) -> FolderKind {
    let normalized = normalize(name.trim());

    if SPECIAL_FOLDERS.iter().any(|s| normalized == *s) {
        return FolderKind::Special;
    }

    let Some(year) = trailing_year(&normalized) else {
        return FolderKind::Album;
    };

    match year_prefix {
        Some(prefix) if folder_prefix(&normalized) != prefix => FolderKind::Album,
        _ => FolderKind::Year(year),
    }
}

/// Derives the prefix this export uses to name its year folders.
///
/// Google calls them `Photos from 2020`, `Foto da 2026`, `Fotos de 2019`: the
/// prefix changes with the account language, but inside a single export it is
/// identical across every year. An album with a year in its name has a prefix of
/// its own, and stays in the minority.
///
/// Returns `None` when there is no clear winner, that is when two different
/// prefixes appear the same number of times: no choice would then be better than
/// a coin toss, and saying so beats deciding.
fn year_prefix(names: &[String]) -> Option<String> {
    let mut conteggi: BTreeMap<String, usize> = BTreeMap::new();
    for name in names {
        let normalized = normalize(name.trim());
        if trailing_year(&normalized).is_some() {
            *conteggi.entry(folder_prefix(&normalized)).or_default() += 1;
        }
    }

    let massimo = *conteggi.values().max()?;
    let mut vincitori = conteggi.iter().filter(|(_, n)| **n == massimo);
    let (prefisso, _) = vincitori.next()?;
    vincitori.next().is_none().then(|| prefisso.clone())
}

/// The part of the name preceding the trailing year, trimmed.
fn folder_prefix(normalized: &str) -> String {
    normalized
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim()
        .to_string()
}

/// Extracts the trailing year of a folder name, if plausible.
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
    // Outside this range it is a number in an album name, not a year.
    (1900..=2100).contains(&year).then_some(year)
}

/// If the name is an edited version, returns the name of the original and the
/// suffix recognised.
///
/// `IMG_1234-edited.jpg` becomes `("IMG_1234.jpg", "-edited")`.
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

        // The cut point has to be searched on the original string, not derived from
        // the length of the normalised one: in NFD `é` takes two characters
        // instead of one, so counting on the NFC form would cut too little and
        // leave pieces of the suffix stuck to the name.
        // The tail cannot be much longer than the suffix in NFC, which keeps the
        // comparison limited to the last few characters.
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

/// Walks a Google Photos export and reconstructs its structure.
pub fn build_index(root: &Path, max_items: usize) -> Result<AlbumIndex> {
    crate::app_state::require_existing(root)?;

    let mut index = AlbumIndex {
        root: root.to_path_buf(),
        ..Default::default()
    };

    // File name -> year folders in which it appears.
    let mut in_years: HashMap<String, PathBuf> = HashMap::new();
    // File name -> albums in which it appears.
    let mut in_albums: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // First pass over the names alone: it tells us what this export calls its
    // years, before deciding what is a year and what is an album.
    let mut nomi: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root)
        .map_err(|e| TakeoutError::io(root, e))?
        .flatten()
    {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') {
            nomi.push(name);
        }
    }
    nomi.sort();

    let prefisso = year_prefix(&nomi);
    if prefisso.is_none() && nomi.iter().filter(|n| classify_folder(n).is_year()).count() > 1 {
        index.warnings.push(Notice::AmbiguousYearFolders);
    }

    for name in nomi {
        let dir = root.join(&name);
        let kind = classify_folder_in(&name, prefisso.as_deref());
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

    // Edited versions are looked for everywhere: they sit beside the original.
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

    // The counts stay complete, the lists do not: they cross the IPC channel
    // on every scan, and on a real library the membership list alone is worth
    // a few megabytes of JSON. The complete manifest is obtained by exporting
    // it, which is the operation made for the purpose.
    //
    // The truncation goes at the end, after every list has been filled: put
    // earlier, it let the edited versions through untouched, since those are
    // collected further down.
    index.membership_count = index.memberships.len();
    index.edited_count = index.edited_pairs.len();
    index.memberships.truncate(max_items);
    index.edited_pairs.truncate(max_items);
    for album in &mut index.albums {
        album.file_count = album.files.len();
        album.files.truncate(max_items);
    }

    if index.album_only > 0 {
        index.warnings.push(Notice::PhotosOnlyInAlbums {
            count: index.album_only,
        });
    }
    if index.membership_count > 0 {
        index.warnings.push(Notice::PhotosSharedWithAlbums {
            count: index.membership_count,
        });
    }

    Ok(index)
}

/// Writes the album manifest.
pub fn export_manifest(root: &Path, destination: &Path) -> Result<ExportReport> {
    // The manifest has to be complete: it is the document that preserves the
    // data, not the preview shown in the interface.
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
    use crate::app_state::testing::{write_bytes, write_file, TempDir, MINIMAL_JPEG};

    #[test]
    fn riconosce_le_cartelle_per_anno_in_piu_lingue() {
        assert_eq!(classify_folder("Photos from 2020"), FolderKind::Year(2020));
        assert_eq!(classify_folder("Foto da 2026"), FolderKind::Year(2026));
        assert_eq!(classify_folder("2019"), FolderKind::Year(2019));
        // A number that is not a year must not fool us.
        assert_eq!(classify_folder("Corsa dei 1000"), FolderKind::Album);
        assert_eq!(classify_folder("Vacanze in Sicilia"), FolderKind::Album);
    }

    #[test]
    fn distingue_un_album_con_l_anno_nel_nome_dalle_annate() {
        let nomi: Vec<String> = [
            "Foto da 2019",
            "Foto da 2020",
            "Foto da 2021",
            "Natale 2024",
            "Vacanze in Sicilia",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let prefisso = year_prefix(&nomi);
        assert_eq!(prefisso.as_deref(), Some("foto da"));

        // Without the export prefix "Christmas 2024" would pass for a year, and
        // its membership would never reach the manifest.
        assert_eq!(classify_folder("Natale 2024"), FolderKind::Year(2024));
        assert_eq!(
            classify_folder_in("Natale 2024", prefisso.as_deref()),
            FolderKind::Album
        );
        assert_eq!(
            classify_folder_in("Foto da 2020", prefisso.as_deref()),
            FolderKind::Year(2020)
        );
        assert_eq!(
            classify_folder_in("Vacanze in Sicilia", prefisso.as_deref()),
            FolderKind::Album
        );

        // A tie makes it impossible to say which prefix belongs to the export:
        // better no answer than one drawn by lot.
        let pari: Vec<String> = ["Foto da 2026", "Natale 2024"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(year_prefix(&pari), None);

        // A single folder ending with a year stays a year: a Google Photos export
        // always has at least one.
        let sola = vec!["Foto da 2026".to_string(), "Matrimonio".to_string()];
        assert_eq!(year_prefix(&sola).as_deref(), Some("foto da"));
    }

    #[test]
    fn segnala_quando_non_riesce_a_distinguere_annate_e_album() {
        let temp = TempDir::new("annate-ambigue");
        let root = temp.path().join("Google Foto");
        // Two different prefixes, one folder each: no winner.
        for cartella in ["Foto da 2026", "Natale 2024"] {
            write_bytes(&root.join(cartella).join("IMG_0001.JPG"), MINIMAL_JPEG);
        }

        let index = build_index(&root, 100).expect("indice");
        assert_eq!(index.year_folders.len(), 2, "when in doubt they stay years");
        assert!(
            index.warnings.contains(&Notice::AmbiguousYearFolders),
            "the ambiguity has to be stated, not hidden: {:?}",
            index.warnings
        );
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
        // A file made only of the suffix has no sensible original.
        assert_eq!(strip_edited_suffix("-edited.jpg"), None);
    }

    /// macOS keeps names in NFD: without normalisation the comparison with the
    /// constant written in NFC would fail, and recognition would work in English
    /// but not in French.
    #[test]
    fn riconosce_i_suffissi_accentati_anche_in_forma_nfd() {
        let nfd: String = "IMG_1-modifie\u{0301}.jpg".to_string();
        assert_ne!(nfd, "IMG_1-modifié.jpg", "the two forms differ in bytes");
        assert_eq!(
            strip_edited_suffix(&nfd).map(|(base, _)| base),
            Some("IMG_1.jpg".to_string())
        );
        assert_eq!(
            strip_edited_suffix("IMG_1-modifié.jpg").map(|(base, _)| base),
            Some("IMG_1.jpg".to_string())
        );
    }

    /// The case that makes this module necessary: the same photo in a year
    /// folder and in an album.
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

        // IMG_1 is in both: the membership has to be recorded.
        let membership = index
            .memberships
            .iter()
            .find(|m| m.file_name == "IMG_1.JPG")
            .expect("appartenenza registrata");
        assert_eq!(membership.albums, vec!["Vacanze in Sicilia"]);
        assert!(membership.canonical.is_some());

        // IMG_2 is only in the album: removing it would make it disappear.
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
