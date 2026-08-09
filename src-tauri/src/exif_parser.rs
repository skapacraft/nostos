// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! EXIF reading and reconciliation with the Google Photos JSON sidecars.
//!
//! The typical problem of a Google Photos export: the capture date and the
//! coordinates live in a `.json` file beside the media, while the EXIF of the
//! exported file is often empty or wrong. This module reads both sources,
//! compares them and prepares the reconciliation.
//!
//! Every operation is read-only until [`apply_metadata`] is explicitly invoked,
//! and that writes only in the mode requested.

use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use exif::{In, Tag, Value};
use little_exif::exif_tag::ExifTag;
use little_exif::ifd::ExifTagGroup;
use little_exif::metadata::Metadata as ExifWriter;
use little_exif::rational::uR64;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::app_state::{trace_dev, Phase, Progress, ProgressSink, Result, TakeoutError};

/// Extensions treated as media in a Google Photos export.
const MEDIA_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "gif", "webp", "tif", "tiff", "dng", "mp4", "mov", "m4v",
    "avi", "3gp", "mkv", "webm",
];

/// Formats whose EXIF tags we know how to rewrite.
///
/// Videos are excluded on purpose: their metadata lives in MP4 atoms or Matroska
/// tags, not in an EXIF block, and handling them here would produce broken
/// files. For those, only the modification date is aligned.
const EXIF_WRITABLE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "heic", "heif", "tif", "tiff", "webp"];

/// Past this threshold the file is not rewritten.
///
/// `little_exif` works on the whole content in memory: without a ceiling, a one
/// gigabyte TIFF multiplied by the active threads would blow up consumption.
/// The photos in a Takeout stay comfortably below it.
const MAX_REWRITE_BYTES: u64 = 128 * 1024 * 1024;

/// Threads used for rewriting.
///
/// The work is dominated by I/O, not by the CPU: past a handful of threads there
/// is nothing to gain, and every extra thread holds its own copy of the file
/// being processed in memory.
const REWRITE_THREADS: usize = 4;

/// Geographic coordinates in decimal degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<f64>,
}

impl GeoPoint {
    /// Google writes `0,0` when the data does not exist: treat it as absent.
    fn is_null_island(&self) -> bool {
        self.latitude.abs() < f64::EPSILON && self.longitude.abs() < f64::EPSILON
    }
}

/// Data read from the file's EXIF.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExifData {
    pub taken_at: Option<DateTime<Utc>>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub geo: Option<GeoPoint>,
    pub orientation: Option<u32>,
}

impl ExifData {
    /// True when the file exposes no useful tag: it happens often to media
    /// reconverted by Google, which lose the original EXIF.
    pub fn is_empty(&self) -> bool {
        self.taken_at.is_none()
            && self.camera_make.is_none()
            && self.camera_model.is_none()
            && self.geo.is_none()
    }
}

/// The `{ "timestamp": "1577880000", "formatted": "..." }` block of the sidecars.
#[derive(Debug, Clone, Deserialize)]
struct TakeoutTime {
    timestamp: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TakeoutGeo {
    latitude: Option<f64>,
    longitude: Option<f64>,
    altitude: Option<f64>,
}

/// The JSON sidecar produced by Google Photos, in the shape we care about.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSidecar {
    title: Option<String>,
    description: Option<String>,
    photo_taken_time: Option<TakeoutTime>,
    creation_time: Option<TakeoutTime>,
    geo_data: Option<TakeoutGeo>,
    geo_data_exif: Option<TakeoutGeo>,
    /// Faces recognised and confirmed by the user in Google Photos.
    people: Option<Vec<RawPerson>>,
    favorited: Option<bool>,
    /// Google's internal counter, with no counterpart in the metadata.
    image_views: Option<String>,
    /// The photo's address on Google Photos, valid only while it exists there.
    url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawPerson {
    name: Option<String>,
}

/// Normalised sidecar.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarData {
    pub path: PathBuf,
    pub title: Option<String>,
    pub description: Option<String>,
    pub taken_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub geo: Option<GeoPoint>,
    /// Names of the recognised faces, in the order Google lists them.
    pub people: Vec<String>,
    pub favorited: bool,
    /// Data Google exports that has no home in the EXIF metadata.
    pub image_views: Option<String>,
    pub url: Option<String>,
}

/// Why a sidecar stays where it is instead of being set aside.
///
/// It is a code, not a sentence: the text is chosen by whoever displays it, in
/// the language they are using. The same holds for the counts the UI groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SidecarKept {
    /// PNG, GIF and video: no EXIF block to write into.
    NoExifContainer,
    /// The file has no readable EXIF block.
    UnreadableExif,
    /// The capture date does not appear to be written into the file yet.
    MissingDate,
    /// The coordinates do not appear to be written into the file yet.
    MissingGeo,
    /// The description does not appear to be written into the file yet.
    MissingDescription,
    /// The recognised faces do not appear to be written into the file yet.
    MissingPeople,
    /// The favourite mark does not appear to be written into the file yet.
    MissingFavorite,
    /// View count: metadata has nowhere to put it.
    ViewCountHasNoTag,
    /// Google Photos address: metadata has nowhere to put it.
    PhotoUrlHasNoTag,
}

impl SidecarData {
    /// Lists the sidecar data that will not end up inside the file.
    ///
    /// It exists so the user can be told what stays behind rather than finding out:
    /// these are counters and addresses internal to Google Photos, not metadata of
    /// the photograph, but the difference is for whoever owns the photos to judge.
    pub fn unwritable(&self) -> Vec<SidecarKept> {
        let mut resto = Vec::new();
        if self.image_views.is_some() {
            resto.push(SidecarKept::ViewCountHasNoTag);
        }
        if self.url.is_some() {
            resto.push(SidecarKept::PhotoUrlHasNoTag);
        }
        resto
    }
}

/// The source chosen for the final value, in decreasing order of reliability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataSource {
    /// Read from the file's tags: it describes the moment of the shot.
    Exif,
    /// Read from the JSON sidecar: it is what the service knows.
    Sidecar,
    /// Derived from the filename, which camera apps generate from the clock.
    FileName,
    Missing,
}

/// Date patterns recognised in the names camera apps generate.
///
/// `A` stands for a digit, every other character has to match exactly. They
/// cover the formats produced by Android, Pixel, iOS, screenshots and messaging:
/// `IMG_20200101_120000.jpg`, `PXL_20200101_120000123.jpg`,
/// `Screenshot_20200101-120000.png`, `signal-2020-01-01-12-00-00.jpg`.
const FILENAME_DATE_PATTERNS: &[&str] = &[
    "AAAA-AA-AA-AA-AA-AA",
    "AAAA_AA_AA_AA_AA_AA",
    "AAAA-AA-AA-AAAAAA",
    "AAAAAAAA-AAAAAA",
    "AAAAAAAA_AAAAAA",
    "AAAAAAAAAAAAAA",
];

/// A unified view of a media file and its metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRecord {
    pub path: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub exif: ExifData,
    pub sidecar: Option<SidecarData>,
    /// Resolved date: EXIF when present, otherwise the sidecar.
    pub resolved_taken_at: Option<DateTime<Utc>>,
    pub taken_at_source: MetadataSource,
    pub resolved_geo: Option<GeoPoint>,
    pub geo_source: MetadataSource,
    /// EXIF is missing but the sidecar has the date: the file can be recovered.
    pub needs_repair: bool,
}

/// Outcome of scanning a photo folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoScanReport {
    pub media_count: usize,
    pub with_sidecar: usize,
    pub with_exif_date: usize,
    pub with_geo: usize,
    pub needs_repair: usize,
    /// Files with no useful EXIF tag at all.
    pub without_exif: usize,
    /// Files whose date was derived from the name, the last resort.
    pub date_from_filename: usize,
    pub total_bytes: u64,
    /// How many files could not be read, complete count.
    pub unreadable_count: usize,
    /// Sample of the problems, truncated for the UI.
    pub unreadable: Vec<String>,
    /// Sample of the first records, for the preview in the UI.
    pub sample: Vec<MediaRecord>,
}

/// How to treat the original files during a repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriteMode {
    /// Touches nothing: it only computes what would change.
    DryRun,
    /// Writes the repaired copies into a separate tree, leaving the originals
    /// untouched. This is the default.
    CopyToOutput,
    /// Rewrites the originals. Has to be requested explicitly.
    InPlace,
}

/// How to lay out the files in the output tree.
///
/// It applies only with [`WriteMode::CopyToOutput`]: the other modes do not
/// choose where to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputLayout {
    /// Recreates the folder structure of the original. This is the default.
    Preserve,
    /// One folder per year.
    ByYear,
    /// One folder per year and one per month.
    ByYearMonth,
    /// Everything in a single folder, ordered by the date in the name.
    Flat,
}

impl OutputLayout {
    /// Destination subfolder for a media file with the given date.
    ///
    /// Anything without a date ends up in a folder of its own rather than being
    /// dumped among the others: a file with no date belongs to no month, and
    /// pretending otherwise would make the ordering a lie.
    fn folder_for(&self, taken_at: Option<DateTime<Utc>>) -> PathBuf {
        use chrono::Datelike;

        match (self, taken_at) {
            (Self::Preserve, _) => PathBuf::new(),
            (_, None) => PathBuf::from("no-date"),
            (Self::ByYear, Some(date)) => PathBuf::from(date.year().to_string()),
            (Self::ByYearMonth, Some(date)) => {
                PathBuf::from(date.year().to_string()).join(format!("{:02}", date.month()))
            }
            (Self::Flat, Some(_)) => PathBuf::new(),
        }
    }
}

/// Repair parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteOptions {
    pub mode: WriteMode,
    /// Layout of the output tree.
    #[serde(default = "default_layout")]
    pub layout: OutputLayout,
    /// Destination root, mandatory with [`WriteMode::CopyToOutput`].
    pub output_root: Option<PathBuf>,
    /// Write date and coordinates into the file's EXIF tags.
    pub write_exif: bool,
    /// Align the file modification date with the capture date.
    pub write_file_times: bool,
}

fn default_layout() -> OutputLayout {
    OutputLayout::Preserve
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            mode: WriteMode::DryRun,
            layout: OutputLayout::Preserve,
            output_root: None,
            write_exif: true,
            write_file_times: true,
        }
    }
}

/// Outcome of the repair.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairReport {
    pub mode: Option<WriteMode>,
    pub output_root: Option<PathBuf>,
    /// Files for which there is a date or a position to write.
    pub candidates: usize,
    pub exif_written: usize,
    pub file_times_written: usize,
    /// Formats whose EXIF we cannot rewrite (video and the like).
    pub skipped_unsupported: usize,
    /// Files past the size threshold.
    pub skipped_too_large: usize,
    /// Sidecars copied beside the files whose EXIF could not be written.
    pub sidecars_copied: usize,
    pub failures: Vec<String>,
}

/// True if the extension belongs to a media type we handle.
pub fn is_media_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| MEDIA_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// True if we know how to rewrite the EXIF tags of this format.
pub fn is_exif_writable(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| EXIF_WRITABLE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Reads the useful EXIF tags. A file without EXIF is not an error: it returns
/// [`ExifData::default`].
pub fn read_exif(path: &Path) -> Result<ExifData> {
    let file = File::open(path).map_err(|e| TakeoutError::io(path, e))?;
    let mut reader = BufReader::new(&file);

    let exif = match exif::Reader::new().read_from_container(&mut reader) {
        Ok(exif) => exif,
        // No EXIF or unsupported format: missing data, not a fault.
        Err(_) => return Ok(ExifData::default()),
    };

    // A file declaring its own offset has to be read with it: without that, local
    // time would be mistaken for UTC and the photo would move through time by the
    // zone difference.
    let offset_minutes = [Tag::OffsetTimeOriginal, Tag::OffsetTimeDigitized]
        .iter()
        .find_map(|tag| exif.get_field(*tag, In::PRIMARY))
        .and_then(|field| ascii_value(&field.value))
        .and_then(|raw| parse_offset(&raw));

    let taken_at = [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
        .iter()
        .find_map(|tag| exif.get_field(*tag, In::PRIMARY))
        .and_then(|field| ascii_value(&field.value))
        .and_then(|raw| parse_exif_datetime(&raw))
        .map(|date| match offset_minutes {
            Some(minutes) => date - chrono::Duration::minutes(minutes as i64),
            None => date,
        });

    let geo = read_gps(&exif).filter(|g| !g.is_null_island());

    Ok(ExifData {
        taken_at,
        camera_make: exif
            .get_field(Tag::Make, In::PRIMARY)
            .and_then(|f| ascii_value(&f.value)),
        camera_model: exif
            .get_field(Tag::Model, In::PRIMARY)
            .and_then(|f| ascii_value(&f.value)),
        geo,
        orientation: exif
            .get_field(Tag::Orientation, In::PRIMARY)
            .and_then(|f| f.value.get_uint(0)),
    })
}

/// Extracts latitude and longitude from the GPS fields, converting from
/// degrees/minutes/seconds to decimal degrees and applying the N/S and E/W ref.
fn read_gps(exif: &exif::Exif) -> Option<GeoPoint> {
    let lat = dms_to_degrees(exif, Tag::GPSLatitude)?;
    let lon = dms_to_degrees(exif, Tag::GPSLongitude)?;

    let lat_ref = exif
        .get_field(Tag::GPSLatitudeRef, In::PRIMARY)
        .and_then(|f| ascii_value(&f.value))
        .unwrap_or_else(|| "N".to_string());
    let lon_ref = exif
        .get_field(Tag::GPSLongitudeRef, In::PRIMARY)
        .and_then(|f| ascii_value(&f.value))
        .unwrap_or_else(|| "E".to_string());

    let latitude = if lat_ref.eq_ignore_ascii_case("S") {
        -lat
    } else {
        lat
    };
    let longitude = if lon_ref.eq_ignore_ascii_case("W") {
        -lon
    } else {
        lon
    };

    let altitude = exif
        .get_field(Tag::GPSAltitude, In::PRIMARY)
        .and_then(|f| match &f.value {
            Value::Rational(v) => v.first().map(|r| r.to_f64()),
            _ => None,
        });

    Some(GeoPoint {
        latitude,
        longitude,
        altitude,
    })
}

fn dms_to_degrees(exif: &exif::Exif, tag: Tag) -> Option<f64> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    match &field.value {
        Value::Rational(values) if values.len() >= 3 => {
            let degrees = values[0].to_f64();
            let minutes = values[1].to_f64();
            let seconds = values[2].to_f64();
            Some(degrees + minutes / 60.0 + seconds / 3600.0)
        }
        _ => None,
    }
}

fn ascii_value(value: &Value) -> Option<String> {
    match value {
        Value::Ascii(entries) => entries
            .first()
            .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
            .filter(|s| !s.is_empty()),
        _ => None,
    }
}

/// Parses an EXIF offset in the form `+01:00`, returning minutes.
fn parse_offset(raw: &str) -> Option<i32> {
    let raw = raw.trim();
    let (sign, rest) = match raw.chars().next()? {
        '+' => (1, &raw[1..]),
        '-' => (-1, &raw[1..]),
        _ => return None,
    };
    let (hours, minutes) = rest.split_once(':')?;
    let total = hours.parse::<i32>().ok()? * 60 + minutes.parse::<i32>().ok()?;
    Some(sign * total)
}

/// EXIF uses `YYYY:MM:DD HH:MM:SS` with no time zone. Without the offset tag it
/// stays ambiguous, and we treat it as UTC: the only choice that does not invent
/// a zone.
fn parse_exif_datetime(raw: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(raw.trim(), "%Y:%m:%d %H:%M:%S")
        .ok()
        .map(|naive| Utc.from_utc_datetime(&naive))
}

fn parse_unix_timestamp(raw: &Option<TakeoutTime>) -> Option<DateTime<Utc>> {
    let seconds: i64 = raw.as_ref()?.timestamp.as_ref()?.parse().ok()?;
    DateTime::from_timestamp(seconds, 0)
}

fn geo_from_takeout(raw: &Option<TakeoutGeo>) -> Option<GeoPoint> {
    let raw = raw.as_ref()?;
    let point = GeoPoint {
        latitude: raw.latitude?,
        longitude: raw.longitude?,
        altitude: raw.altitude,
    };
    (!point.is_null_island()).then_some(point)
}

/// Derives the capture date from the filename.
///
/// Camera apps write the clock time into the name, so when EXIF and sidecar are
/// both missing this remains the only source that is not invented. The reading
/// is deliberately strict: the date has to be valid as a real date, otherwise
/// any serial number would turn into a timestamp.
pub fn parse_date_from_filename(file_name: &str) -> Option<DateTime<Utc>> {
    let bytes: Vec<char> = file_name.chars().collect();

    for pattern in FILENAME_DATE_PATTERNS {
        let pattern: Vec<char> = pattern.chars().collect();
        if bytes.len() < pattern.len() {
            continue;
        }

        for start in 0..=(bytes.len() - pattern.len()) {
            let window = &bytes[start..start + pattern.len()];
            let matches = window.iter().zip(pattern.iter()).all(|(c, p)| {
                if *p == 'A' {
                    c.is_ascii_digit()
                } else {
                    c == p
                }
            });
            if !matches {
                continue;
            }

            // A date preceded by other digits is part of a longer number: that is
            // the case with serial numbers, and it has to be discarded.
            if start > 0 && bytes[start - 1].is_ascii_digit() {
                continue;
            }

            // Afterwards, a few digits are legitimate: Pixel phones write the
            // milliseconds at the end (`PXL_20200101_120000123`). We tolerate up
            // to three; past that, we are inside a number again.
            let trailing = bytes[start + pattern.len()..]
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .count();
            if trailing > 3 {
                continue;
            }

            let digits: String = window.iter().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() != 14 {
                continue;
            }
            if let Some(parsed) = build_datetime(&digits) {
                return Some(parsed);
            }
        }
    }

    None
}

/// Builds a date from fourteen digits `YYYYMMDDhhmmss`, validating it.
fn build_datetime(digits: &str) -> Option<DateTime<Utc>> {
    let year: i32 = digits[0..4].parse().ok()?;
    let month: u32 = digits[4..6].parse().ok()?;
    let day: u32 = digits[6..8].parse().ok()?;
    let hour: u32 = digits[8..10].parse().ok()?;
    let minute: u32 = digits[10..12].parse().ok()?;
    let second: u32 = digits[12..14].parse().ok()?;

    // A photograph taken in 1823 or in 2400 is a false positive.
    if !(1900..=2100).contains(&year) {
        return None;
    }

    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let time = chrono::NaiveTime::from_hms_opt(hour, minute, second)?;
    Some(Utc.from_utc_datetime(&date.and_time(time)))
}

/// Locates and reads the JSON sidecar associated with the media.
pub fn read_sidecar(media: &Path, index: Option<&FileIndex>) -> Result<Option<SidecarData>> {
    let Some(sidecar_path) = find_sidecar(media, index) else {
        return Ok(None);
    };

    let content =
        std::fs::read_to_string(&sidecar_path).map_err(|e| TakeoutError::io(&sidecar_path, e))?;
    let raw: RawSidecar = serde_json::from_str(&content)
        .map_err(|e| TakeoutError::Metadata(format!("{}: {e}", sidecar_path.display())))?;

    Ok(Some(SidecarData {
        path: sidecar_path,
        title: raw.title,
        description: raw.description.filter(|d| !d.is_empty()),
        taken_at: parse_unix_timestamp(&raw.photo_taken_time),
        created_at: parse_unix_timestamp(&raw.creation_time),
        geo: geo_from_takeout(&raw.geo_data).or_else(|| geo_from_takeout(&raw.geo_data_exif)),
        people: raw
            .people
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| p.name)
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect(),
        favorited: raw.favorited.unwrap_or(false),
        // Google writes "0" even when there is nothing to say: a counter at zero
        // is not data anyone loses.
        image_views: raw.image_views.filter(|v| v != "0" && !v.is_empty()),
        url: raw.url.filter(|u| !u.is_empty()),
    }))
}

/// Maximum length of a sidecar name produced by Google, `.json` included.
/// compreso.
/// This is behaviour observed in exports, not a rule documented by Google: if
/// an export were to use a different limit, long names would come back without
/// a sidecar and the scan would say so by counting the files whose date is
/// derived from the name.
///
const MAX_SIDECAR_NAME: usize = 46;

const JSON_EXT: &str = ".json";

/// Cuts a string to `limit` characters without splitting a UTF-8 character.
///
/// Counting in bytes would break accented letters and ideographs, and filenames
/// are full of both.
fn tronca(testo: &str, limite: usize) -> &str {
    match testo.char_indices().nth(limite) {
        Some((fine, _)) => &testo[..fine],
        None => testo,
    }
}

/// Builds the candidate names of the sidecar.
///
/// Google has changed schema several times and shortens long names, so there is
/// no single rule: we try the known variants in order of frequency.
fn sidecar_candidates(media: &Path) -> Vec<PathBuf> {
    let Some(file_name) = media.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    let parent = media.parent().unwrap_or_else(|| Path::new("."));
    let stem = media
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);

    let mut candidates = vec![
        // Classic schema: IMG_0001.JPG.json
        parent.join(format!("{file_name}.json")),
        // Recent schema: IMG_0001.JPG.supplemental-metadata.json
        parent.join(format!("{file_name}.supplemental-metadata.json")),
        // Variant without the media extension: IMG_0001.json
        parent.join(format!("{stem}.json")),
    ];

    // Long names: Google shortens the sidecar name until it fits in
    // MAX_SIDECAR_NAME characters, `.json` included. The cut almost always
    // falls inside `.supplemental-metadata`, which turns up truncated
    // (`.supplemental-me.json`) or disappears entirely, leaving the media name
    // itself cut short. Without these variants a photo with a long name comes
    // out with no sidecar and its date ends up derived from the name, which is
    // the last resort and carries no coordinates.
    for base in [
        format!("{file_name}.supplemental-metadata"),
        file_name.to_string(),
    ] {
        // The count is in characters, not bytes, for the same reason `tronca`
        // cuts on boundaries: an accented name must not produce a different
        // candidate merely because it takes more bytes.
        if base.chars().count() + JSON_EXT.len() <= MAX_SIDECAR_NAME {
            continue;
        }
        let tagliato = tronca(&base, MAX_SIDECAR_NAME - JSON_EXT.len());
        let candidato = parent.join(format!("{tagliato}{JSON_EXT}"));
        if !candidates.contains(&candidato) {
            candidates.push(candidato);
        }
    }

    // Duplicate files: `IMG_0001(1).JPG` has sidecar `IMG_0001.JPG(1).json`.
    if let Some(open) = stem.rfind('(') {
        if stem.ends_with(')') {
            let base = &stem[..open];
            let counter = &stem[open..];
            let extension = media
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_default();
            candidates.push(parent.join(format!("{base}{extension}{counter}.json")));
        }
    }

    candidates
}

/// The set of existing paths, gathered once during the scan.
///
/// It exists so the filesystem is not queried for every sidecar candidate: the
/// version calling `is_file()` on three or four paths per photo paid a `stat`
/// inside folders holding tens of thousands of entries, and the cost of the scan
/// grew with the square of the library instead of in proportion to it.
///
pub type FileIndex = std::collections::HashSet<PathBuf>;

/// True if the path exists, consulting the index when available.
fn exists(path: &Path, index: Option<&FileIndex>) -> bool {
    match index {
        Some(index) => index.contains(path),
        None => path.is_file(),
    }
}

fn find_sidecar(media: &Path, index: Option<&FileIndex>) -> Option<PathBuf> {
    let candidates = sidecar_candidates(media);
    let found = candidates.iter().find(|c| exists(c, index)).cloned();

    if found.is_none() {
        // The most frequent cause of a "repair that does nothing" is a sidecar
        // that exists under a name we did not anticipate: listing the attempts
        // makes the problem obvious rather than silent.
        trace_dev!(
            "no sidecar for {}: tried {}",
            media.file_name().unwrap_or_default().to_string_lossy(),
            candidates
                .iter()
                .map(|c| c
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    found
}

/// Builds the unified record of a single media file.
pub fn inspect_media(path: &Path, index: Option<&FileIndex>) -> Result<MediaRecord> {
    let metadata = std::fs::metadata(path).map_err(|e| TakeoutError::io(path, e))?;
    let exif = read_exif(path)?;
    let sidecar = read_sidecar(path, index)?;

    let sidecar_taken = sidecar.as_ref().and_then(|s| s.taken_at.or(s.created_at));

    // Order of reliability: EXIF describes the moment of the shot, the sidecar
    // what the service knows, the filename what the camera app wrote. The name
    // steps in only when the other two are silent.
    let from_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(parse_date_from_filename);

    let (resolved_taken_at, taken_at_source) = match (exif.taken_at, sidecar_taken, from_name) {
        (Some(date), _, _) => (Some(date), MetadataSource::Exif),
        (None, Some(date), _) => (Some(date), MetadataSource::Sidecar),
        (None, None, Some(date)) => (Some(date), MetadataSource::FileName),
        (None, None, None) => (None, MetadataSource::Missing),
    };

    let sidecar_geo = sidecar.as_ref().and_then(|s| s.geo);
    let (resolved_geo, geo_source) = match (exif.geo, sidecar_geo) {
        (Some(geo), _) => (Some(geo), MetadataSource::Exif),
        (None, Some(geo)) => (Some(geo), MetadataSource::Sidecar),
        (None, None) => (None, MetadataSource::Missing),
    };

    trace_dev!(
        "read {}: sidecar={} exif_data={} date={} ({:?}) gps={} ({:?})",
        path.file_name().unwrap_or_default().to_string_lossy(),
        sidecar
            .as_ref()
            .map(|s| s
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned())
            .unwrap_or_else(|| "absent".to_string()),
        if exif.is_empty() { "empty" } else { "present" },
        resolved_taken_at
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| "none".to_string()),
        taken_at_source,
        resolved_geo
            .map(|g| format!("{:.5},{:.5}", g.latitude, g.longitude))
            .unwrap_or_else(|| "none".to_string()),
        geo_source,
    );

    Ok(MediaRecord {
        file_name: path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        path: path.to_path_buf(),
        size_bytes: metadata.len(),
        needs_repair: exif.taken_at.is_none() && resolved_taken_at.is_some(),
        exif,
        sidecar,
        resolved_taken_at,
        taken_at_source,
        resolved_geo,
        geo_source,
    })
}

/// Walks a Google Photos folder and summarises the state of the metadata.
pub fn scan_directory(root: &Path, sample_size: usize) -> Result<PhotoScanReport> {
    crate::app_state::require_existing(root)?;

    let mut report = PhotoScanReport::default();

    // A single pass collects the media paths and the index of everything that
    // exists, so the sidecar search never goes back to the filesystem.
    let mut index = FileIndex::new();
    let mut media: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        if is_media_file(entry.path()) {
            media.push(entry.path().to_path_buf());
        }
        index.insert(entry.into_path());
    }

    // Reading is dominated by opening two files per photo, the media and its
    // sidecar: that is I/O work, and on a large library a single thread becomes
    // the bottleneck. The thread count matches the one used for rewriting, so
    // concurrent reads on the disk are not multiplied.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(
            REWRITE_THREADS.min(std::thread::available_parallelism().map_or(1, |n| n.get())),
        )
        .build()
        .map_err(|e| TakeoutError::Task(e.to_string()))?;

    let esiti: Vec<std::result::Result<MediaRecord, (PathBuf, String)>> = pool.install(|| {
        media
            .par_iter()
            .map(|path| {
                inspect_media(path, Some(&index)).map_err(|err| (path.clone(), err.to_string()))
            })
            .collect()
    });

    // Aggregation stays sequential and in pass order, so the sample shown is
    // always the same for a given library.
    for esito in esiti {
        match esito {
            Ok(record) => {
                report.media_count += 1;
                report.total_bytes += record.size_bytes;
                if record.sidecar.is_some() {
                    report.with_sidecar += 1;
                }
                if record.exif.taken_at.is_some() {
                    report.with_exif_date += 1;
                }
                if record.exif.is_empty() {
                    report.without_exif += 1;
                }
                if record.taken_at_source == MetadataSource::FileName {
                    report.date_from_filename += 1;
                }
                if record.resolved_geo.is_some() {
                    report.with_geo += 1;
                }
                if record.needs_repair {
                    report.needs_repair += 1;
                }
                if report.sample.len() < sample_size {
                    report.sample.push(record);
                }
            }
            Err((path, err)) => {
                report.unreadable_count += 1;
                // The list is truncated but the count is not: sending the frontend
                // one string per unreadable file would mean megabytes of JSON on a
                // library in bad shape.
                if report.unreadable.len() < sample_size {
                    report.unreadable.push(format!(
                        "{}: {err}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
            }
        }
    }

    Ok(report)
}

/// Converts decimal degrees into the degrees/minutes/seconds triple EXIF wants.
///
/// The seconds are expressed in thousandths: with a denominator of 1 we would
/// lose about thirty metres of precision.
fn degrees_to_dms(value: f64) -> Vec<uR64> {
    let value = value.abs();
    let degrees = value.trunc();
    let minutes_total = (value - degrees) * 60.0;
    let minutes = minutes_total.trunc();
    let seconds = (minutes_total - minutes) * 60.0;

    vec![
        uR64 {
            nominator: degrees as u32,
            denominator: 1,
        },
        uR64 {
            nominator: minutes as u32,
            denominator: 1,
        },
        uR64 {
            nominator: (seconds * 1000.0).round() as u32,
            denominator: 1000,
        },
    ]
}

/// The time zone finder, built once.
///
/// Building it loads the zone polygons into memory: doing that for every photo
/// would make repairs unusable on a large library.
static TIMEZONE_FINDER: std::sync::OnceLock<tzf_rs::DefaultFinder> = std::sync::OnceLock::new();

/// Converts a UTC instant into the local time of the place where it was taken.
///
/// Returns the time to write and the offset in `+01:00` form.
///
/// This function exists because `DateTimeOriginal` **is not UTC**: the EXIF
/// specification defines it as the local time on the camera clock at the moment
/// of the shot. The Google sidecar, on the other hand, carries a UTC instant.
/// Writing that instant without conversion moves every photo back by the zone
/// difference: a shot taken in Milan at 14:00 would show up at 13:00.
fn local_time_and_offset(instant: DateTime<Utc>, geo: Option<GeoPoint>) -> (NaiveDateTime, String) {
    use chrono::Offset;

    let Some(geo) = geo else {
        // Without coordinates the zone is unknown. We write the UTC instant and
        // declare that it is UTC: it stays a different time from the clock of
        // whoever took the shot, but the file is not ambiguous and no program
        // reads it wrong.
        return (instant.naive_utc(), "+00:00".to_string());
    };

    let finder = TIMEZONE_FINDER.get_or_init(tzf_rs::DefaultFinder::new);
    let name = finder.get_tz_name(geo.longitude, geo.latitude);

    let Ok(zone) = name.parse::<chrono_tz::Tz>() else {
        return (instant.naive_utc(), "+00:00".to_string());
    };

    let local = instant.with_timezone(&zone);
    let seconds = local.offset().fix().local_minus_utc();
    let sign = if seconds < 0 { '-' } else { '+' };
    let abs = seconds.abs();

    (
        local.naive_local(),
        format!("{sign}{:02}:{:02}", abs / 3600, (abs % 3600) / 60),
    )
}

/// Applies the resolved EXIF tags to the file given.
///
/// The file passed here is always a working copy, never the original: the
/// caller creates it and swaps it in atomically.
fn write_exif_tags(target: &Path, record: &MediaRecord) -> Result<()> {
    // A file without an EXIF block is not an error: we start from empty metadata
    // and `little_exif` inserts the missing segment.
    let mut writer = ExifWriter::new_from_path(target).unwrap_or_else(|_| ExifWriter::new());

    if let Some(taken_at) = record.resolved_taken_at {
        // The time has to be written as the clock on the spot read it, with the
        // offset beside it: that is what the EXIF specification asks for and what
        // photo management programs expect to find.
        let (local, offset) = local_time_and_offset(taken_at, record.resolved_geo);
        let formatted = local.format("%Y:%m:%d %H:%M:%S").to_string();

        writer.set_tag(ExifTag::DateTimeOriginal(formatted.clone()));
        writer.set_tag(ExifTag::CreateDate(formatted));
        writer.set_tag(ExifTag::OffsetTimeOriginal(offset.clone()));
        writer.set_tag(ExifTag::OffsetTimeDigitized(offset));
    }

    if let Some(geo) = record.resolved_geo {
        writer.set_tag(ExifTag::GPSVersionID(vec![2, 3, 0, 0]));
        writer.set_tag(ExifTag::GPSLatitude(degrees_to_dms(geo.latitude)));
        writer.set_tag(ExifTag::GPSLatitudeRef(
            if geo.latitude < 0.0 { "S" } else { "N" }.to_string(),
        ));
        writer.set_tag(ExifTag::GPSLongitude(degrees_to_dms(geo.longitude)));
        writer.set_tag(ExifTag::GPSLongitudeRef(
            if geo.longitude < 0.0 { "W" } else { "E" }.to_string(),
        ));

        if let Some(altitude) = geo.altitude {
            // The reference distinguishes above (0) from below (1) sea level: EXIF
            // altitude is an unsigned value.
            writer.set_tag(ExifTag::GPSAltitudeRef(vec![u8::from(altitude < 0.0)]));
            writer.set_tag(ExifTag::GPSAltitude(vec![uR64 {
                nominator: (altitude.abs() * 100.0).round() as u32,
                denominator: 100,
            }]));
        }
    }

    // Everything the sidecar carries beyond date and coordinates. Without this
    // step the description the user wrote, the faces they confirmed and the fact
    // that a photo was a favourite would stay in the JSON, which is to say they
    // would be lost the moment the JSON is left behind: exactly the problem this
    // application exists to solve.
    if let Some(sidecar) = &record.sidecar {
        if let Some(description) = sidecar.description.as_deref() {
            writer.set_tag(ExifTag::ImageDescription(description.to_string()));
            // Windows and several photo management programs read the proprietary
            // field instead of `ImageDescription`: writing both costs a few dozen
            // bytes and stops the description turning up empty depending on the
            // program.
            writer.set_tag(windows_string_tag(XP_COMMENT, description));
        }

        if !sidecar.people.is_empty() {
            // There is no EXIF tag for faces. `XPKeywords` is the home the rest of
            // the ecosystem actually reads, separated by semicolons.
            writer.set_tag(windows_string_tag(XP_KEYWORDS, &sidecar.people.join(";")));
        }

        if sidecar.favorited {
            // The Google Photos star becomes the top rating, which is the
            // convention Windows, Lightroom and digiKam follow.
            writer.set_tag(ExifTag::UnknownINT16U(
                vec![5],
                RATING,
                ExifTagGroup::GENERIC,
            ));
            writer.set_tag(ExifTag::UnknownINT16U(
                vec![99],
                RATING_PERCENT,
                ExifTagGroup::GENERIC,
            ));
        }
    }

    writer
        .write_to_file(target)
        .map_err(|e| TakeoutError::io(target, e))
}

/// Lists what the sidecar holds and the file does not yet carry with it.
///
/// It is the check that makes moving the JSON safe: while this list is not
/// empty, that sidecar is the only copy of something and has to stay where it
/// is. The comparison looks inside the file instead of trusting the outcome the
/// repair reported, because the file is what will be left to the user.
///
/// The data listed by [`SidecarData::unwritable`] does not appear here: there
/// is no tag to put it in, so waiting for it would keep the list non-empty
/// forever.
pub fn sidecar_residual(media: &Path, sidecar: &SidecarData) -> Result<Vec<SidecarKept>> {
    if !is_exif_writable(media) {
        return Ok(vec![SidecarKept::NoExifContainer]);
    }

    let file = std::fs::File::open(media).map_err(|e| TakeoutError::io(media, e))?;
    let mut reader = std::io::BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return Ok(vec![SidecarKept::UnreadableExif]);
    };

    let mut mancanti = Vec::new();

    if let Some(atteso) = sidecar.taken_at {
        let scritta = [Tag::DateTimeOriginal, Tag::DateTimeDigitized]
            .iter()
            .find_map(|tag| exif.get_field(*tag, In::PRIMARY))
            .and_then(|field| ascii_value(&field.value))
            .and_then(|raw| parse_exif_datetime(&raw));
        let offset = [Tag::OffsetTimeOriginal, Tag::OffsetTimeDigitized]
            .iter()
            .find_map(|tag| exif.get_field(*tag, In::PRIMARY))
            .and_then(|field| ascii_value(&field.value))
            .and_then(|raw| parse_offset(&raw));

        let coincide = scritta
            .map(|data| match offset {
                Some(minuti) => data - chrono::Duration::minutes(minuti as i64),
                None => data,
            })
            .is_some_and(|data| (data - atteso).num_seconds().abs() <= 1);

        if !coincide {
            mancanti.push(SidecarKept::MissingDate);
        }
    }

    if sidecar.geo.is_some() && read_gps(&exif).filter(|g| !g.is_null_island()).is_none() {
        mancanti.push(SidecarKept::MissingGeo);
    }

    if sidecar.description.is_some() && exif.get_field(Tag::ImageDescription, In::PRIMARY).is_none()
    {
        mancanti.push(SidecarKept::MissingDescription);
    }

    if !sidecar.people.is_empty() && !has_tag(&exif, XP_KEYWORDS) {
        mancanti.push(SidecarKept::MissingPeople);
    }

    if sidecar.favorited && !has_tag(&exif, RATING) {
        mancanti.push(SidecarKept::MissingFavorite);
    }

    Ok(mancanti)
}

/// True if the tag exists and carries a non-empty value.
///
/// The proprietary tags have no constant in `kamadak-exif`, so they are built
/// from their numeric code in the TIFF context, which is where they live.
///
fn has_tag(exif: &exif::Exif, code: u16) -> bool {
    exif.get_field(Tag(exif::Context::Tiff, code), In::PRIMARY)
        .is_some_and(|field| !field.value.display_as(field.tag).to_string().is_empty())
}

/// Microsoft proprietary tags, absent from the EXIF specification but written
/// and read by most photo management programs.
const XP_COMMENT: u16 = 0x9C9C;
const XP_KEYWORDS: u16 = 0x9C9E;
/// Rating of the photo, in stars and as a percentage.
const RATING: u16 = 0x4746;
const RATING_PERCENT: u16 = 0x4749;

/// Encodes a string in the form the `XP*` tags expect.
///
/// They are declared as byte sequences but hold little endian UTF-16 including
/// the terminator: writing UTF-8 into them produces unreadable text, and
/// omitting the terminator makes stray characters appear at the end.
fn windows_string_tag(code: u16, value: &str) -> ExifTag {
    let mut bytes = Vec::with_capacity(value.len() * 2 + 2);
    for unit in value.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes.extend_from_slice(&[0, 0]);
    ExifTag::UnknownINT8U(bytes, code, ExifTagGroup::GENERIC)
}

/// Picks a free name in the folder given.
///
/// The reservation is centralised because rewriting runs in parallel: two
/// threads finding the same free name at the same instant would produce a
/// single file.
fn unique_target(
    folder: &Path,
    file_name: &str,
    taken: &Mutex<HashSet<PathBuf>>,
) -> Result<PathBuf> {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let extension = Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    let mut guard = taken
        .lock()
        .map_err(|_| TakeoutError::Task("the name registry is unavailable".to_string()))?;

    let mut candidate = folder.join(file_name);
    let mut counter = 2;
    while guard.contains(&candidate) || candidate.exists() {
        candidate = folder.join(format!("{stem} ({counter}){extension}"));
        counter += 1;
    }
    guard.insert(candidate.clone());
    Ok(candidate)
}

/// Outcome of writing a single media file.
#[derive(Debug, Default)]
struct WriteOutcome {
    exif_written: bool,
    times_written: bool,
    too_large: bool,
    sidecar_copied: bool,
}

/// Writes the metadata of a single media file according to the options given.
fn repair_one(
    record: &MediaRecord,
    root: &Path,
    options: &WriteOptions,
    taken: &Mutex<HashSet<PathBuf>>,
) -> Result<WriteOutcome> {
    let source = record.path.as_path();

    let destination = match options.mode {
        WriteMode::InPlace => source.to_path_buf(),
        WriteMode::CopyToOutput => {
            let output_root = options.output_root.as_ref().ok_or_else(|| {
                TakeoutError::Metadata("the destination folder is missing".to_string())
            })?;

            match options.layout {
                // Original structure: the relative path is already unique.
                OutputLayout::Preserve => {
                    let relative = source.strip_prefix(root).unwrap_or(source);
                    output_root.join(relative)
                }
                // When reorganising by date, files with the same name coming from
                // different folders end up together: without a counter the second
                // would silently overwrite the first.
                layout => {
                    let folder = output_root.join(layout.folder_for(record.resolved_taken_at));
                    unique_target(&folder, &record.file_name, taken)?
                }
            }
        }
        WriteMode::DryRun => return Ok(WriteOutcome::default()),
    };

    let writable = options.write_exif && is_exif_writable(source);
    let too_large = record.size_bytes > MAX_REWRITE_BYTES;

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TakeoutError::io(parent, e))?;
    }

    let mut exif_written = false;

    if writable && !too_large {
        // We always work on a temporary file in the same folder as the
        // destination, so the final swap is an atomic rename on the same
        // filesystem. If something goes wrong halfway, the original was never
        // touched and only a temporary is left to remove.
        // The temporary has to keep the original extension: `little_exif`
        // derives the container format from precisely that, and a trailing
        // `.oth-tmp` would make it fail.
        let file_name = destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let temp = destination.with_file_name(format!(".oth-tmp-{file_name}"));

        std::fs::copy(source, &temp).map_err(|e| TakeoutError::io(&temp, e))?;

        match write_exif_tags(&temp, record) {
            Ok(()) => {
                std::fs::rename(&temp, &destination)
                    .map_err(|e| TakeoutError::io(&destination, e))?;
                exif_written = true;
                trace_dev!("EXIF written: {}", destination.display());
            }
            Err(err) => {
                let _ = std::fs::remove_file(&temp);
                trace_dev!("EXIF failed on {}: {err}", record.file_name);
                return Err(err);
            }
        }
    } else {
        if too_large {
            trace_dev!(
                "{} skipped: {} bytes past the rewrite limit",
                record.file_name,
                record.size_bytes
            );
        } else {
            trace_dev!(
                "{}: format without EXIF writing, only the file date is aligned",
                record.file_name
            );
        }

        if options.mode == WriteMode::CopyToOutput {
            // Format not rewritable or too large: the copy has to be produced
            // anyway, otherwise the output tree would be incomplete.
            std::fs::copy(source, &destination).map_err(|e| TakeoutError::io(&destination, e))?;
        }
    }

    // With no EXIF written, the date survives only in the file modification
    // date: the most fragile metadata there is, wiped by a cloud upload or a
    // copy without `-p`. The sidecar is the only durable source left, so it
    // goes beside the file instead of being left behind.
    let mut sidecar_copied = false;
    if options.mode == WriteMode::CopyToOutput && !exif_written {
        if let Some(sidecar) = record.sidecar.as_ref() {
            let sidecar_target = destination.with_file_name(
                sidecar
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
            std::fs::copy(&sidecar.path, &sidecar_target)
                .map_err(|e| TakeoutError::io(&sidecar_target, e))?;
            sidecar_copied = true;
            trace_dev!(
                "sidecar kept beside {}: {}",
                record.file_name,
                sidecar_target
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            );
        }
    }

    let mut times_written = false;
    if options.write_file_times {
        if let Some(taken_at) = record.resolved_taken_at {
            let stamp = filetime::FileTime::from_unix_time(taken_at.timestamp(), 0);
            filetime::set_file_mtime(&destination, stamp)
                .map_err(|e| TakeoutError::io(&destination, e))?;
            times_written = true;
        }
    }

    Ok(WriteOutcome {
        exif_written,
        times_written,
        too_large,
        sidecar_copied,
    })
}

/// Repairs the metadata of the media under `root`.
///
/// The default is non-destructive: without instructions to the contrary no
/// original file is modified. The work runs in parallel but with a limited
/// thread count, because it is dominated by I/O and because every thread holds
/// its own working copy in memory.
pub fn apply_metadata(
    root: &Path,
    options: &WriteOptions,
    progress: ProgressSink<'_>,
) -> Result<RepairReport> {
    crate::app_state::require_existing(root)?;

    if options.mode == WriteMode::CopyToOutput {
        let output_root = options.output_root.as_ref().ok_or_else(|| {
            TakeoutError::Metadata("copy mode requires a destination folder".to_string())
        })?;
        // Writing the output inside the source would create a recursion on the
        // next pass and mix originals with copies.
        if output_root.starts_with(root) {
            return Err(TakeoutError::Metadata(
                "the destination folder cannot sit inside the source one".to_string(),
            ));
        }

        // The repaired copy duplicates the library: better to say so now than
        // to fill the disk halfway through.
        let da_scrivere: u64 = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        crate::app_state::require_free_space(output_root, da_scrivere)?;
    }

    let mut index = FileIndex::new();
    let mut media: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        if is_media_file(entry.path()) {
            media.push(entry.path().to_path_buf());
        }
        index.insert(entry.into_path());
    }

    let total = media.len();
    trace_dev!(
        "repair started: {total} media in {}, mode {:?}, output {}",
        root.display(),
        options.mode,
        options
            .output_root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    progress(Progress::new(Phase::Scanning, 0, total, 0));

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(
            REWRITE_THREADS.min(std::thread::available_parallelism().map_or(1, |n| n.get())),
        )
        .build()
        .map_err(|e| TakeoutError::Task(e.to_string()))?;

    let done = std::sync::atomic::AtomicUsize::new(0);
    let errors = std::sync::atomic::AtomicUsize::new(0);
    let taken: Mutex<HashSet<PathBuf>> = Mutex::new(HashSet::new());
    use std::sync::atomic::Ordering::Relaxed;

    let outcomes: Vec<PerFile> = pool.install(|| {
        media
            .par_iter()
            .map(|path| {
                let outcome = process_media(path, root, options, &taken, &index);
                if outcome.failure.is_some() {
                    errors.fetch_add(1, Relaxed);
                }

                let seen = done.fetch_add(1, Relaxed) + 1;
                progress(
                    Progress::new(Phase::Writing, seen, total, errors.load(Relaxed))
                        .with_current(path.file_name().unwrap_or_default().to_string_lossy()),
                );

                outcome
            })
            .collect()
    });

    let mut report = RepairReport {
        mode: Some(options.mode),
        output_root: options.output_root.clone(),
        ..Default::default()
    };

    for outcome in outcomes {
        report.candidates += usize::from(outcome.is_candidate);
        report.exif_written += usize::from(outcome.exif_written);
        report.file_times_written += usize::from(outcome.times_written);
        report.skipped_unsupported += usize::from(outcome.unsupported);
        report.skipped_too_large += usize::from(outcome.too_large);
        report.sidecars_copied += usize::from(outcome.sidecar_copied);
        if let Some(failure) = outcome.failure {
            report.failures.push(failure);
        }
    }

    trace_dev!(
        "repair finished: {} candidates, {} EXIF written, {} dates aligned, {} sidecars kept, {} unsupported, {} errors",
        report.candidates,
        report.exif_written,
        report.file_times_written,
        report.sidecars_copied,
        report.skipped_unsupported,
        report.failures.len()
    );

    progress(Progress::new(
        Phase::Done,
        total,
        total,
        report.failures.len(),
    ));

    Ok(report)
}

/// Outcome of processing a single file, aggregated at the end of the run.
#[derive(Debug, Default)]
struct PerFile {
    is_candidate: bool,
    exif_written: bool,
    times_written: bool,
    unsupported: bool,
    too_large: bool,
    sidecar_copied: bool,
    failure: Option<String>,
}

/// Reads a media file and applies the repair, never propagating a panic.
///
/// A single unreadable file must not stop the processing of another ten
/// thousand photos: the error is recorded and the run carries on.
fn process_media(
    path: &Path,
    root: &Path,
    options: &WriteOptions,
    taken: &Mutex<HashSet<PathBuf>>,
    index: &FileIndex,
) -> PerFile {
    let record = match inspect_media(path, Some(index)) {
        Ok(record) => record,
        Err(err) => {
            return PerFile {
                failure: Some(format!("{}: {err}", path.display())),
                ..Default::default()
            }
        }
    };

    // "Candidate" means there is something to write. It does not mean the file
    // should be ignored: in copy mode the output tree has to contain the whole
    // library, including the photos that had nothing to repair. Leaving them
    // out would produce a copy that looks complete and is not.
    let is_candidate = record.resolved_taken_at.is_some() || record.resolved_geo.is_some();
    if !is_candidate && options.mode != WriteMode::CopyToOutput {
        return PerFile::default();
    }

    let unsupported = options.write_exif && !is_exif_writable(path);

    match repair_one(&record, root, options, taken) {
        Ok(outcome) => PerFile {
            is_candidate,
            exif_written: outcome.exif_written,
            times_written: outcome.times_written,
            unsupported: unsupported && !outcome.too_large,
            too_large: outcome.too_large,
            sidecar_copied: outcome.sidecar_copied,
            ..Default::default()
        },
        Err(err) => PerFile {
            is_candidate,
            failure: Some(format!("{}: {err}", path.display())),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app_state::testing::{write_bytes, write_file, TempDir, MINIMAL_JPEG};

    #[test]
    fn interpreta_il_formato_data_exif() {
        let parsed = parse_exif_datetime("2020:01:01 12:00:00").expect("data valida");
        assert_eq!(parsed.timestamp(), 1_577_880_000);
        assert!(parse_exif_datetime("2020-01-01 12:00:00").is_none());
    }

    #[test]
    fn scarta_le_coordinate_a_zero() {
        let point = GeoPoint {
            latitude: 0.0,
            longitude: 0.0,
            altitude: None,
        };
        assert!(point.is_null_island());
    }

    #[test]
    fn genera_i_nomi_sidecar_attesi() {
        let candidates = sidecar_candidates(Path::new("/foto/IMG_0001.JPG"));
        assert!(candidates.contains(&PathBuf::from("/foto/IMG_0001.JPG.json")));
        assert!(candidates.contains(&PathBuf::from(
            "/foto/IMG_0001.JPG.supplemental-metadata.json"
        )));
    }

    #[test]
    fn gestisce_i_nomi_sidecar_troncati() {
        // Short name: no truncation, and no pointless extra candidate.
        let corti = sidecar_candidates(Path::new("/foto/IMG_0001.JPG"));
        assert!(
            corti.iter().all(|c| {
                let nome = c.file_name().unwrap().to_string_lossy();
                nome.ends_with(".json") && nome.chars().count() <= MAX_SIDECAR_NAME
            }),
            "a short name must not produce a truncated candidate"
        );

        // Medium name: it fits in 46 characters on its own, but not with the whole
        // `.supplemental-metadata` appended, which therefore turns up truncated.
        let medi = sidecar_candidates(Path::new("/foto/PXL_20260115_120000123.jpg"));
        assert!(
            medi.contains(&PathBuf::from(
                "/foto/PXL_20260115_120000123.jpg.supplemental-m.json"
            )),
            "expected the truncated suffix, found {medi:?}"
        );

        // Long name: the cut falls inside the media name itself.
        let lunghi = sidecar_candidates(Path::new(
            "/foto/Foto scattata durante la gita del 04-01-2022.jpg",
        ));
        assert!(
            lunghi.contains(&PathBuf::from(
                "/foto/Foto scattata durante la gita del 04-01-2.json"
            )),
            "expected the cut name, found {lunghi:?}"
        );

        // The cut is counted in characters: an accented name must not produce a
        // shorter candidate merely because it takes more bytes.
        let accentati = sidecar_candidates(Path::new(
            "/foto/Foto della città più bella del mondo intero.jpg",
        ));
        let tagliato = accentati
            .iter()
            .find(|c| {
                let nome = c.file_name().unwrap().to_string_lossy();
                !nome.contains("supplemental") && nome.chars().count() == MAX_SIDECAR_NAME
            })
            .unwrap_or_else(|| panic!("no truncated candidate in {accentati:?}"));
        assert_eq!(
            tagliato.file_name().unwrap().to_string_lossy(),
            "Foto della città più bella del mondo inte.json"
        );
    }

    #[test]
    fn gestisce_i_duplicati_con_contatore() {
        let candidates = sidecar_candidates(Path::new("/foto/IMG_0001(1).JPG"));
        assert!(candidates.contains(&PathBuf::from("/foto/IMG_0001.JPG(1).json")));
    }

    #[test]
    fn riconosce_solo_le_estensioni_media() {
        assert!(is_media_file(Path::new("a.HEIC")));
        assert!(is_media_file(Path::new("a.mp4")));
        assert!(!is_media_file(Path::new("a.json")));
    }

    #[test]
    fn scrive_exif_solo_sui_contenitori_che_lo_prevedono() {
        assert!(is_exif_writable(Path::new("foto.jpg")));
        assert!(is_exif_writable(Path::new("foto.HEIC")));
        assert!(is_exif_writable(Path::new("scansione.tiff")));
        // Videos keep their metadata in container atoms, not in EXIF.
        assert!(!is_exif_writable(Path::new("clip.mp4")));
        assert!(!is_exif_writable(Path::new("clip.mov")));
    }

    /// The sidecar carries more than date and coordinates, and the rest belongs in the file.
    ///
    /// If description, faces and favourite stayed only in the JSON, moving the
    /// JSON would lose them: which is to say the very fault this application
    /// exists to repair would reappear, with one extra step.
    #[test]
    fn porta_dentro_al_file_anche_descrizione_volti_e_preferito() {
        let temp = TempDir::new("sidecar-completo");
        let foto = temp.path().join("IMG_0001.JPG");
        write_bytes(&foto, MINIMAL_JPEG);
        write_file(
            &temp.path().join("IMG_0001.JPG.json"),
            r#"{
              "title": "IMG_0001.JPG",
              "description": "Cena sul lago con Anna",
              "photoTakenTime": { "timestamp": "1577880000" },
              "geoData": { "latitude": 45.4642, "longitude": 9.19, "altitude": 0.0 },
              "people": [{ "name": "Anna Bianchi" }, { "name": "Luca Verdi" }],
              "favorited": true,
              "imageViews": "42",
              "url": "https://photos.google.com/photo/AF1QipN"
            }"#,
        );

        let record = inspect_media(&foto, None).expect("lettura media");
        let sidecar = record.sidecar.as_ref().expect("sidecar letto");
        assert_eq!(sidecar.people, ["Anna Bianchi", "Luca Verdi"]);
        assert!(sidecar.favorited);
        assert_eq!(
            sidecar.unwritable(),
            [
                SidecarKept::ViewCountHasNoTag,
                SidecarKept::PhotoUrlHasNoTag
            ],
            "what does not fit in the file has to be nameable"
        );

        let report = apply_metadata(
            temp.path(),
            &WriteOptions {
                mode: WriteMode::InPlace,
                ..WriteOptions::default()
            },
            &crate::app_state::no_progress,
        )
        .expect("riparazione");
        assert_eq!(report.exif_written, 1);

        let bytes = std::fs::read(&foto).expect("rilettura");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "it stays a valid JPEG");

        // `ImageDescription` is plain text and is found exactly as written.
        assert!(
            String::from_utf8_lossy(&bytes).contains("Cena sul lago con Anna"),
            "the description has to end up inside the file"
        );

        // The `XP*` tags are little endian UTF-16: looking for the UTF-8 form
        // would find nothing even if the write had succeeded.
        let utf16: Vec<u8> = "Anna Bianchi;Luca Verdi"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert!(
            bytes.windows(utf16.len()).any(|f| f == utf16),
            "faces go into XPKeywords, separated by semicolons"
        );

        // A full rating is how the rest of the ecosystem reads the Google Photos
        // star.
        let riletto = read_exif(&foto).expect("rilettura EXIF");
        assert!(
            riletto.taken_at.is_some(),
            "the date must not be lost while writing the rest"
        );
    }

    /// This test protects two different things with the same assertion.
    ///
    /// The first is about quality: in PNG the EXIF lives in the `eXIf` chunk,
    /// which viewers read patchily, so writing it would give the user the
    /// illusion of having repaired something.
    ///
    /// The second is about security: inside `little_exif` the XML parser is
    /// reachable only from the PNG path, and that parser has two known DoS
    /// issues (RUSTSEC-2026-0194 and -0195) with no fixed version available.
    /// While PNG stays out, that code is never executed. The two
    /// vulnerabilities are listed in `deny.toml` on the basis of precisely this
    /// constraint: if anyone adds "png" here, the exception lapses and this test
    /// has to fail as a reminder.
    #[test]
    fn png_resta_fuori_dalla_scrittura_exif() {
        assert!(
            !EXIF_WRITABLE_EXTENSIONS.contains(&"png"),
            "aggiungere PNG riattiva il parser XML vulnerabile di little_exif: \
             rivedere prima l'eccezione in deny.toml"
        );
        assert!(!is_exif_writable(Path::new("immagine.png")));
    }

    #[test]
    fn deduce_la_data_dai_nomi_generati_dalle_fotocamere() {
        let attesa = 1_577_880_000; // 2020-01-01 12:00:00 UTC
        for nome in [
            "IMG_20200101_120000.jpg",
            "VID_20200101_120000.mp4",
            "PXL_20200101_120000123.jpg",
            "Screenshot_20200101-120000.png",
            "IMG-20200101-WA0001.jpeg",
            "signal-2020-01-01-12-00-00.jpg",
            "2020-01-01-120000.heic",
            "20200101120000.jpg",
        ] {
            let parsed = parse_date_from_filename(nome);
            // `IMG-20200101-WA0001` carries no valid time: it has to be rejected.
            if nome.contains("WA0001") {
                assert!(parsed.is_none(), "{nome} carries no real time");
                continue;
            }
            assert_eq!(
                parsed.map(|d| d.timestamp()),
                Some(attesa),
                "name not recognised: {nome}"
            );
        }
    }

    #[test]
    fn non_scambia_numeri_qualsiasi_per_date() {
        // Impossible date: month 13.
        assert!(parse_date_from_filename("IMG_20201301_120000.jpg").is_none());
        // Impossible hour.
        assert!(parse_date_from_filename("IMG_20200101_250000.jpg").is_none());
        // Year out of range.
        assert!(parse_date_from_filename("IMG_18000101_120000.jpg").is_none());
        // A long serial number is not a date.
        assert!(parse_date_from_filename("DSC000202001011200001234.jpg").is_none());
        assert!(parse_date_from_filename("IMG_1234.jpg").is_none());
    }

    #[test]
    fn il_nome_interviene_solo_dopo_exif_e_sidecar() {
        // The order is verified on the data: EXIF beats sidecar, sidecar beats
        // name. Here it is enough to establish that the name is the last resort.
        assert_eq!(MetadataSource::Exif as u8, 0);
        assert!(parse_date_from_filename("IMG_20200101_120000.jpg").is_some());
    }

    /// `DateTimeOriginal` is the time on the clock at the place, not UTC. The
    /// Google sidecar carries a UTC instant instead: writing it without
    /// conversion would move every photo back by the zone difference.
    #[test]
    fn converte_listante_utc_nellora_locale_del_luogo() {
        let istante = DateTime::from_timestamp(1_577_880_000, 0).expect("istante"); // 12:00 UTC

        // Milan in January: CET, one hour ahead.
        let milano = GeoPoint {
            latitude: 45.4642,
            longitude: 9.19,
            altitude: None,
        };
        let (locale, offset) = local_time_and_offset(istante, Some(milano));
        assert_eq!(locale.format("%H:%M").to_string(), "13:00");
        assert_eq!(offset, "+01:00");

        // New York on the same date: five hours behind, and the day changes only
        // when the hour requires it.
        let new_york = GeoPoint {
            latitude: 40.7128,
            longitude: -74.006,
            altitude: None,
        };
        let (locale, offset) = local_time_and_offset(istante, Some(new_york));
        assert_eq!(locale.format("%H:%M").to_string(), "07:00");
        assert_eq!(offset, "-05:00");

        // India: a half-hour offset, the case that breaks implementations
        // assuming whole-hour offsets.
        let delhi = GeoPoint {
            latitude: 28.6139,
            longitude: 77.209,
            altitude: None,
        };
        let (_, offset) = local_time_and_offset(istante, Some(delhi));
        assert_eq!(offset, "+05:30");

        // Without coordinates we invent no zone: we declare UTC.
        let (locale, offset) = local_time_and_offset(istante, None);
        assert_eq!(locale.format("%H:%M").to_string(), "12:00");
        assert_eq!(offset, "+00:00");
    }

    #[test]
    fn tiene_conto_dellora_legale() {
        // Same place, July: CEST, two hours ahead instead of one.
        let luglio = DateTime::from_timestamp(1_593_604_800, 0).expect("istante"); // 2020-07-01 12:00 UTC
        let milano = GeoPoint {
            latitude: 45.4642,
            longitude: 9.19,
            altitude: None,
        };
        let (locale, offset) = local_time_and_offset(luglio, Some(milano));
        assert_eq!(locale.format("%H:%M").to_string(), "14:00");
        assert_eq!(offset, "+02:00");
    }

    #[test]
    fn interpreta_loffset_dichiarato_nei_file() {
        assert_eq!(parse_offset("+01:00"), Some(60));
        assert_eq!(parse_offset("-05:00"), Some(-300));
        assert_eq!(parse_offset("+05:30"), Some(330));
        assert_eq!(parse_offset("00:00"), None, "manca il segno");
        assert_eq!(parse_offset("boh"), None);
    }

    #[test]
    fn converte_i_gradi_decimali_in_gradi_primi_secondi() {
        // 45.4642 degrees correspond to 45° 27' 51.12".
        let dms = degrees_to_dms(45.4642);
        assert_eq!(dms[0].nominator, 45);
        assert_eq!(dms[1].nominator, 27);
        assert_eq!(dms[2].denominator, 1000);
        assert_eq!(dms[2].nominator, 51_120);

        // The sign is carried by the N/S and E/W reference, not by the value.
        let sud = degrees_to_dms(-45.4642);
        assert_eq!(sud[0].nominator, 45);
    }
}
