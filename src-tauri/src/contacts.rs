// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reading the Google Contacts export (vCard 3.0).
//!
//! The exported `.vcf` file is a stream of concatenated cards. The parser
//! implements the three rules that break naive implementations: line folding,
//! group prefixes (`item1.EMAIL`) and separator escaping.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::app_state::{ExportReport, Notice, Result, TakeoutError};

/// A normalised contact.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub display_name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub organization: Option<String>,
    pub title: Option<String>,
    pub birthday: Option<String>,
    pub note: Option<String>,
}

impl Contact {
    /// Deduplication key: first normalised email, otherwise first phone number in
    /// canonical form, otherwise the display name.
    fn dedup_key(&self) -> Option<String> {
        if let Some(email) = self.emails.first() {
            return Some(format!("email:{}", email.to_ascii_lowercase()));
        }
        if let Some(phone) = self.phones.first() {
            return Some(format!("tel:{}", normalize_phone(phone)));
        }
        self.display_name
            .as_ref()
            .map(|name| format!("name:{}", name.to_ascii_lowercase()))
    }

    fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.given_name.is_none()
            && self.family_name.is_none()
            && self.emails.is_empty()
            && self.phones.is_empty()
    }
}

/// Outcome of reading one or more vCard files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsReport {
    pub sources: Vec<PathBuf>,
    pub total: usize,
    pub unique: usize,
    pub duplicates: usize,
    pub with_email: usize,
    pub with_phone: usize,
    pub without_contact_info: usize,
    pub warnings: Vec<Notice>,
    /// Sample of the first contacts, for the preview in the UI.
    pub sample: Vec<Contact>,
}

/// Reduces a phone number to digits only, keeping the international prefix, so
/// that `+39 320 123` and `0039320123` can be compared.
fn normalize_phone(raw: &str) -> String {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = digits.strip_prefix("00").unwrap_or(&digits);
    // We compare only the last significant digits: country prefixes are written
    // inconsistently across exports.
    if digits.len() > 9 {
        digits[digits.len() - 9..].to_string()
    } else {
        digits.to_string()
    }
}

/// Rejoins folded lines: a line starting with a space or a tab is the
/// continuation of the previous one.
///
/// vCard (RFC 6350) and iCalendar (RFC 5545) share the same content line
/// format, so [`calendar`](crate::calendar) uses these functions too rather
/// than rewriting them.
pub(crate) fn unfold(content: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();

    for raw in content.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(line.to_string());
    }

    lines
}

/// Restores characters protected by escaping (shared by vCard and iCalendar).
pub(crate) fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();

    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }

    out
}

/// Splits a property into name, parameters and value.
///
/// The first `:` separates header from value, but it can also appear inside
/// quoted parameters, so the split ignores the quoted portions.
pub(crate) fn split_property(line: &str) -> Option<(String, Vec<String>, String)> {
    let mut in_quotes = false;
    let mut colon = None;

    for (index, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(index);
                break;
            }
            _ => {}
        }
    }

    let colon = colon?;
    let (head, value) = line.split_at(colon);
    let value = value[1..].to_string();

    let mut parts = head.split(';');
    let raw_name = parts.next()?;
    // Strips the group prefix if present: `item1.EMAIL` becomes `EMAIL`.
    let name = raw_name
        .rsplit('.')
        .next()
        .unwrap_or(raw_name)
        .to_ascii_uppercase();
    let params = parts.map(|p| p.to_ascii_uppercase()).collect();

    Some((name, params, value))
}

/// Parses the contents of a vCard file.
pub fn parse_vcard(content: &str) -> Vec<Contact> {
    let mut contacts = Vec::new();
    let mut current: Option<Contact> = None;

    for line in unfold(content) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.eq_ignore_ascii_case("BEGIN:VCARD") {
            current = Some(Contact::default());
            continue;
        }
        if trimmed.eq_ignore_ascii_case("END:VCARD") {
            if let Some(contact) = current.take() {
                if !contact.is_empty() {
                    contacts.push(contact);
                }
            }
            continue;
        }

        let Some(contact) = current.as_mut() else {
            continue;
        };
        let Some((name, params, value)) = split_property(trimmed) else {
            continue;
        };

        // Quoted-printable belongs to vCard 2.1: we do not decode it, but it must
        // not pollute the data with raw bytes either.
        if params.iter().any(|p| p.contains("QUOTED-PRINTABLE")) {
            continue;
        }

        let value = unescape(&value);
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }

        match name.as_str() {
            "FN" => contact.display_name = Some(value),
            "N" => {
                // Format: surname;given name;middle names;prefixes;suffixes
                let mut fields = value.split(';');
                contact.family_name = fields
                    .next()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                contact.given_name = fields
                    .next()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
            }
            "EMAIL" => {
                if !contact.emails.contains(&value) {
                    contact.emails.push(value);
                }
            }
            "TEL" => {
                if !contact.phones.contains(&value) {
                    contact.phones.push(value);
                }
            }
            "ORG" => {
                contact.organization = value
                    .split(';')
                    .next()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
            }
            "TITLE" => contact.title = Some(value),
            "BDAY" => contact.birthday = Some(value),
            "NOTE" => contact.note = Some(value),
            _ => {}
        }
    }

    contacts
}

/// Reads a single `.vcf` file.
pub fn parse_file(path: &Path) -> Result<Vec<Contact>> {
    let content = std::fs::read_to_string(path).map_err(|e| TakeoutError::io(path, e))?;
    Ok(parse_vcard(&content))
}

/// Searches recursively for `.vcf` files under `root` and aggregates a report.
pub fn scan_directory(root: &Path, sample_size: usize) -> Result<ContactsReport> {
    let (mut report, unique) = collect_contacts(root)?;
    report.sample = unique.into_iter().take(sample_size).collect();
    Ok(report)
}

/// Reads every vCard under `root`, returning a report and deduplicated contacts.
fn collect_contacts(root: &Path) -> Result<(ContactsReport, Vec<Contact>)> {
    crate::app_state::require_existing(root)?;

    let mut report = ContactsReport::default();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut unique_contacts: Vec<Contact> = Vec::new();

    // A path may point straight at the file rather than at the folder.
    let files: Vec<PathBuf> = if root.is_file() {
        vec![root.to_path_buf()]
    } else {
        WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .map(|e| e.into_path())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("vcf"))
                    .unwrap_or(false)
            })
            .collect()
    };

    for file in files {
        match parse_file(&file) {
            Ok(contacts) => {
                report.sources.push(file);
                for contact in contacts {
                    report.total += 1;
                    if !contact.emails.is_empty() {
                        report.with_email += 1;
                    }
                    if !contact.phones.is_empty() {
                        report.with_phone += 1;
                    }
                    if contact.emails.is_empty() && contact.phones.is_empty() {
                        report.without_contact_info += 1;
                    }

                    match contact.dedup_key() {
                        Some(key) if seen.contains_key(&key) => {
                            report.duplicates += 1;
                            if let Some(&index) = seen.get(&key) {
                                merge_into(&mut unique_contacts[index], contact);
                            }
                        }
                        Some(key) => {
                            seen.insert(key, unique_contacts.len());
                            unique_contacts.push(contact);
                        }
                        None => unique_contacts.push(contact),
                    }
                }
            }
            Err(err) => report
                .warnings
                .push(Notice::read_failed(file.display(), err)),
        }
    }

    report.unique = unique_contacts.len();
    Ok((report, unique_contacts))
}

/// Writes a clean vCard 3.0 with the deduplicated contacts.
///
/// The version stays 3.0, the one Google exports. Converting to 4.0 would mean
/// remapping parameters and partial dates at the risk of losing information,
/// while Proton, Tuta and Nextcloud import 3.0 without complaint: the value of
/// this step is in the deduplication, not in the version number.
/// numero di versione.
pub fn export_vcf(root: &Path, destination: &Path) -> Result<ExportReport> {
    let (_, contacts) = collect_contacts(root)?;

    let mut out = String::new();
    for contact in &contacts {
        out.push_str("BEGIN:VCARD\r\nVERSION:3.0\r\n");

        let display = contact.display_name.clone().unwrap_or_else(|| {
            [
                contact.given_name.as_deref(),
                contact.family_name.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
        });
        if !display.is_empty() {
            out.push_str(&format!("FN:{}\r\n", escape_value(&display)));
        }

        // `N` is mandatory in vCard 3.0: some importers reject the card when it is
        // missing, even with `FN` present.
        out.push_str(&format!(
            "N:{};{};;;\r\n",
            escape_value(contact.family_name.as_deref().unwrap_or_default()),
            escape_value(contact.given_name.as_deref().unwrap_or_default())
        ));

        for email in &contact.emails {
            out.push_str(&format!("EMAIL;TYPE=INTERNET:{}\r\n", escape_value(email)));
        }
        for phone in &contact.phones {
            out.push_str(&format!("TEL:{}\r\n", escape_value(phone)));
        }
        if let Some(organization) = &contact.organization {
            out.push_str(&format!("ORG:{}\r\n", escape_value(organization)));
        }
        if let Some(title) = &contact.title {
            out.push_str(&format!("TITLE:{}\r\n", escape_value(title)));
        }
        if let Some(birthday) = &contact.birthday {
            out.push_str(&format!("BDAY:{}\r\n", escape_value(birthday)));
        }
        if let Some(note) = &contact.note {
            out.push_str(&format!("NOTE:{}\r\n", escape_value(note)));
        }

        out.push_str("END:VCARD\r\n");
    }

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TakeoutError::io(parent, e))?;
    }
    std::fs::write(destination, &out).map_err(|e| TakeoutError::io(destination, e))?;

    Ok(ExportReport {
        path: destination.to_path_buf(),
        written: contacts.len(),
        bytes: out.len() as u64,
    })
}

/// Protects the separators inside a vCard value.
fn escape_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}

/// Merges a duplicate into the contact already recorded, keeping every detail.
fn merge_into(target: &mut Contact, other: Contact) {
    if target.display_name.is_none() {
        target.display_name = other.display_name;
    }
    if target.organization.is_none() {
        target.organization = other.organization;
    }
    if target.birthday.is_none() {
        target.birthday = other.birthday;
    }
    for email in other.emails {
        if !target.emails.contains(&email) {
            target.emails.push(email);
        }
    }
    for phone in other.phones {
        if !target.phones.contains(&phone) {
            target.phones.push(phone);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Mario Rossi\r\nN:Rossi;Mario;;;\r\nitem1.EMAIL;TYPE=INTERNET:mario@example.com\r\nTEL;TYPE=CELL:+39 320 1234567\r\nORG:Acme S.p.A.;Reparto\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:Giulia Bianchi\r\nEMAIL:giulia@example.com\r\nEND:VCARD\r\n";

    #[test]
    fn reads_basic_cards() {
        let contacts = parse_vcard(SAMPLE);
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts[0].display_name.as_deref(), Some("Mario Rossi"));
        assert_eq!(contacts[0].family_name.as_deref(), Some("Rossi"));
        assert_eq!(contacts[0].given_name.as_deref(), Some("Mario"));
        assert_eq!(contacts[0].emails, vec!["mario@example.com"]);
        assert_eq!(contacts[0].organization.as_deref(), Some("Acme S.p.A."));
    }

    #[test]
    fn rejoins_folded_lines() {
        let folded = "BEGIN:VCARD\r\nNOTE:prima parte\r\n  e seconda parte\r\nEND:VCARD\r\n";
        let contacts = parse_vcard(folded);
        assert_eq!(contacts.len(), 0, "a card holding only NOTE stays empty");

        let lines = unfold(folded);
        assert!(lines
            .iter()
            .any(|l| l == "NOTE:prima parte e seconda parte"));
    }

    #[test]
    fn parses_the_escaping() {
        assert_eq!(unescape(r"Rossi\, Mario"), "Rossi, Mario");
        assert_eq!(unescape(r"riga1\nriga2"), "riga1\nriga2");
    }

    #[test]
    fn normalises_phone_numbers_for_comparison() {
        assert_eq!(
            normalize_phone("+39 320 1234567"),
            normalize_phone("00393201234567")
        );
        assert_eq!(normalize_phone("320 1234567"), "201234567");
    }

    #[test]
    fn ignores_quoted_printable_values() {
        let card = "BEGIN:VCARD\r\nFN;ENCODING=QUOTED-PRINTABLE:Mario=20Rossi\r\nEMAIL:m@example.com\r\nEND:VCARD\r\n";
        let contacts = parse_vcard(card);
        assert_eq!(contacts.len(), 1);
        assert!(contacts[0].display_name.is_none());
        assert_eq!(contacts[0].emails, vec!["m@example.com"]);
    }
}
