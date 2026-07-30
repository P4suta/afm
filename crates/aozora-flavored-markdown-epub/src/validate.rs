//! Phase 2 — validate manifest invariants and XML 1.0 character data before
//! rendering or writing any package output.

use std::path::Path;

use crate::discover::{Manuscript, Metadata};
use crate::{Error, Result};

pub(crate) fn validate(manuscript: &Manuscript) -> Result<()> {
    // XML representability is the outer contract. Report a prohibited
    // scalar with its path/field even when the same value would also fail a
    // semantic rule such as "non-empty title" or BCP 47 language syntax.
    validate_xml_text(
        &manuscript.metadata_path,
        "title",
        &manuscript.metadata.title,
    )?;
    validate_xml_text(
        &manuscript.metadata_path,
        "creator",
        &manuscript.metadata.creator,
    )?;
    validate_xml_text(
        &manuscript.metadata_path,
        "language",
        &manuscript.metadata.language,
    )?;
    if let Some(identifier) = &manuscript.metadata.identifier {
        validate_xml_text(&manuscript.metadata_path, "identifier", identifier)?;
    }
    validate_metadata(&manuscript.metadata)
}

pub(crate) fn validate_metadata(meta: &Metadata) -> Result<()> {
    if meta.title.trim().is_empty() {
        return Err(Error::MetadataInvalid {
            field: "title",
            reason: "dc:title must be a non-empty string".to_owned(),
        });
    }
    if meta.creator.trim().is_empty() {
        return Err(Error::MetadataInvalid {
            field: "creator",
            reason: "dc:creator must be a non-empty string".to_owned(),
        });
    }
    if !is_bcp47_subset(&meta.language) {
        return Err(Error::MetadataInvalid {
            field: "language",
            reason: format!(
                "dc:language must be a BCP 47 tag (e.g. `ja`, `ja-JP`); got {:?}",
                meta.language
            ),
        });
    }
    Ok(())
}

pub(crate) fn validate_xml_text(path: &Path, field: &'static str, text: &str) -> Result<()> {
    if let Some((byte_offset, ch)) = text.char_indices().find(|&(_, ch)| !is_xml10(ch)) {
        return Err(Error::XmlCharacter {
            path: path.to_path_buf(),
            field,
            byte_offset,
            codepoint: ch.into(),
        });
    }
    Ok(())
}

const fn is_xml10(ch: char) -> bool {
    matches!(ch, '\u{9}' | '\u{A}' | '\u{D}')
        || matches!(ch as u32, 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x0001_0000..=0x0010_FFFF)
}

pub(crate) fn is_bcp47_subset(tag: &str) -> bool {
    if tag.is_empty() {
        return false;
    }
    let mut subtags = tag.split('-');
    let primary = subtags.next().unwrap_or_default();
    if !(2..=3).contains(&primary.len()) || !primary.bytes().all(|byte| byte.is_ascii_alphabetic())
    {
        return false;
    }
    subtags.all(|sub| {
        (2..=8).contains(&sub.len()) && sub.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_10_accepts_whitespace_and_unicode_but_rejects_controls_and_noncharacters() {
        for valid in ["\t\n\r ", "本文", "𠮷"] {
            validate_xml_text(Path::new("source.md"), "chapter", valid).unwrap();
        }
        for invalid in ["\0", "\u{B}", "\u{1F}", "\u{FFFE}", "\u{FFFF}"] {
            let err = validate_xml_text(Path::new("source.md"), "chapter", invalid).unwrap_err();
            assert!(matches!(err, Error::XmlCharacter { .. }), "{err:?}");
        }
    }
}
