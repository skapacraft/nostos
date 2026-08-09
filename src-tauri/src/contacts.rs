// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Lettura dell'export Contatti di Google (vCard 3.0).
//!
//! Il file `.vcf` esportato è un flusso di schede concatenate. Il parser
//! implementa le tre regole che rompono le implementazioni ingenue: il line
//! folding, i prefissi di gruppo (`item1.EMAIL`) e l'escaping dei separatori.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::app_state::{ExportReport, Result, TakeoutError};

/// Un contatto normalizzato.
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
    /// Chiave di deduplica: prima email normalizzata, altrimenti primo telefono
    /// in forma canonica, altrimenti il nome visualizzato.
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

/// Esito della lettura di uno o più file vCard.
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
    pub warnings: Vec<String>,
    /// Campione dei primi contatti, per l'anteprima nella UI.
    pub sample: Vec<Contact>,
}

/// Riduce un numero di telefono a sole cifre, tenendo il prefisso
/// internazionale, per poter confrontare `+39 320 123` e `0039320123`.
fn normalize_phone(raw: &str) -> String {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = digits.strip_prefix("00").unwrap_or(&digits);
    // Confrontiamo solo le ultime cifre significative: i prefissi nazionali
    // sono scritti in modo incoerente negli export.
    if digits.len() > 9 {
        digits[digits.len() - 9..].to_string()
    } else {
        digits.to_string()
    }
}

/// Ricompone le righe spezzate: una riga che inizia con spazio o tabulazione è
/// la continuazione della precedente.
///
/// vCard (RFC 6350) e iCalendar (RFC 5545) condividono lo stesso formato di
/// content line, quindi anche [`calendar`](crate::calendar) usa queste
/// funzioni invece di riscriverle.
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

/// Ripristina i caratteri protetti dall'escaping (comune a vCard e iCalendar).
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

/// Divide una proprietà in nome, parametri e valore.
///
/// Il primo `:` separa intestazione e valore, ma può comparire dentro i
/// parametri quotati, quindi il taglio ignora le porzioni tra virgolette.
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
    // Rimuove l'eventuale prefisso di gruppo: `item1.EMAIL` diventa `EMAIL`.
    let name = raw_name
        .rsplit('.')
        .next()
        .unwrap_or(raw_name)
        .to_ascii_uppercase();
    let params = parts.map(|p| p.to_ascii_uppercase()).collect();

    Some((name, params, value))
}

/// Interpreta il contenuto di un file vCard.
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

        // Il quoted-printable appartiene a vCard 2.1: non lo decodifichiamo,
        // ma non deve nemmeno inquinare i dati con byte grezzi.
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
                // Formato: cognome;nome;secondi nomi;prefissi;suffissi
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

/// Legge un singolo file `.vcf`.
pub fn parse_file(path: &Path) -> Result<Vec<Contact>> {
    let content = std::fs::read_to_string(path).map_err(|e| TakeoutError::io(path, e))?;
    Ok(parse_vcard(&content))
}

/// Cerca ricorsivamente i file `.vcf` sotto `root` e li aggrega in un report.
pub fn scan_directory(root: &Path, sample_size: usize) -> Result<ContactsReport> {
    let (mut report, unique) = collect_contacts(root)?;
    report.sample = unique.into_iter().take(sample_size).collect();
    Ok(report)
}

/// Legge tutti i vCard sotto `root`, restituendo report e contatti deduplicati.
fn collect_contacts(root: &Path) -> Result<(ContactsReport, Vec<Contact>)> {
    crate::app_state::require_existing(root)?;

    let mut report = ContactsReport::default();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut unique_contacts: Vec<Contact> = Vec::new();

    // Un percorso può puntare direttamente al file invece che alla cartella.
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
            Err(err) => report.warnings.push(err.to_string()),
        }
    }

    report.unique = unique_contacts.len();
    Ok((report, unique_contacts))
}

/// Scrive un vCard 3.0 pulito con i contatti deduplicati.
///
/// La versione resta la 3.0, la stessa che Google esporta. Convertire a 4.0
/// significherebbe rimappare parametri e date parziali con il rischio di
/// perdere informazioni, mentre Proton, Tuta e Nextcloud importano la 3.0
/// senza obiezioni: il valore di questo passaggio sta nella deduplica, non nel
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

        // `N` è obbligatorio in vCard 3.0: alcuni importatori rifiutano la
        // scheda se manca, anche quando `FN` è presente.
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

/// Protegge i separatori in un valore vCard.
fn escape_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}

/// Fonde un duplicato nel contatto già registrato, senza perdere recapiti.
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
    fn legge_le_schede_di_base() {
        let contacts = parse_vcard(SAMPLE);
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts[0].display_name.as_deref(), Some("Mario Rossi"));
        assert_eq!(contacts[0].family_name.as_deref(), Some("Rossi"));
        assert_eq!(contacts[0].given_name.as_deref(), Some("Mario"));
        assert_eq!(contacts[0].emails, vec!["mario@example.com"]);
        assert_eq!(contacts[0].organization.as_deref(), Some("Acme S.p.A."));
    }

    #[test]
    fn ricompone_le_righe_spezzate() {
        let folded = "BEGIN:VCARD\r\nNOTE:prima parte\r\n  e seconda parte\r\nEND:VCARD\r\n";
        let contacts = parse_vcard(folded);
        assert_eq!(contacts.len(), 0, "una scheda con solo NOTE resta vuota");

        let lines = unfold(folded);
        assert!(lines
            .iter()
            .any(|l| l == "NOTE:prima parte e seconda parte"));
    }

    #[test]
    fn interpreta_lescaping() {
        assert_eq!(unescape(r"Rossi\, Mario"), "Rossi, Mario");
        assert_eq!(unescape(r"riga1\nriga2"), "riga1\nriga2");
    }

    #[test]
    fn normalizza_i_numeri_per_il_confronto() {
        assert_eq!(
            normalize_phone("+39 320 1234567"),
            normalize_phone("00393201234567")
        );
        assert_eq!(normalize_phone("320 1234567"), "201234567");
    }

    #[test]
    fn ignora_i_valori_quoted_printable() {
        let card = "BEGIN:VCARD\r\nFN;ENCODING=QUOTED-PRINTABLE:Mario=20Rossi\r\nEMAIL:m@example.com\r\nEND:VCARD\r\n";
        let contacts = parse_vcard(card);
        assert_eq!(contacts.len(), 1);
        assert!(contacts[0].display_name.is_none());
        assert_eq!(contacts[0].emails, vec!["m@example.com"]);
    }
}
