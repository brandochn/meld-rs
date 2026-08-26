//! File encoding detection and conversion.
//!
//! Uses the `encoding_rs` crate to detect text encoding and convert
//! between encodings. Replaces the Python `iohelpers.py` encoding logic.

use encoding_rs::Encoding;

/// Errors that can occur during encoding operations.
#[derive(Debug, thiserror::Error)]
pub enum EncodingError {
    /// The file could not be read.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// The encoding could not be detected.
    #[error("Could not detect encoding")]
    DetectionFailed,

    /// The encoding is not supported.
    #[error("Unsupported encoding: {0}")]
    Unsupported(String),
}

/// A list of candidate encodings to try, in order of preference.
const CANDIDATE_ENCODINGS: &[&str] = &[
    "UTF-8",
    "ISO-8859-1",
    "Windows-1252",
    "UTF-16LE",
    "UTF-16BE",
    "Shift_JIS",
    "GBK",
    "EUC-JP",
    "EUC-KR",
];

/// Read a file with automatic encoding detection.
///
/// Tries each candidate encoding until one succeeds without errors.
pub fn read_with_encoding(path: &str) -> Result<String, EncodingError> {
    let bytes = std::fs::read(path)?;

    for encoding_name in CANDIDATE_ENCODINGS {
        if let Some(encoding) = Encoding::for_label(encoding_name.as_bytes()) {
            let (text, _, had_errors) = encoding.decode(&bytes);
            if !had_errors {
                return Ok(text.into_owned());
            }
        }
    }

    // Fall back to UTF-8 with replacement characters
    let (text, _, _) = Encoding::for_label(b"UTF-8")
        .unwrap_or(encoding_rs::UTF_8)
        .decode(&bytes);
    Ok(text.into_owned())
}

/// Read a file and return the decoded text together with the name of the
/// encoding that was used.
///
/// When `preferred` is `Some`, that encoding is tried first and kept when it
/// decodes without errors, so callers can preserve the encoding a pane was
/// originally loaded with (e.g. on revert). Otherwise the encoding is
/// auto-detected.
pub fn read_text_with_encoding(
    path: &str,
    preferred: Option<&str>,
) -> Result<(String, String), EncodingError> {
    let bytes = std::fs::read(path)?;

    if let Some(name) = preferred {
        if let Some(encoding) = Encoding::for_label(name.as_bytes()) {
            let (text, _, had_errors) = encoding.decode(&bytes);
            if !had_errors {
                return Ok((text.into_owned(), name.to_string()));
            }
        }
    }

    let encoding = detect_encoding(&bytes);
    let (text, _, _) = encoding.decode(&bytes);
    Ok((text.into_owned(), encoding.name().to_string()))
}

/// Write a file using the specified encoding.
///
/// If `encoding_name` is `None`, UTF-8 is used.
pub fn write_with_encoding(
    path: &str,
    text: &str,
    encoding_name: Option<&str>,
) -> Result<(), EncodingError> {
    let encoding = encoding_name
        .and_then(|n| Encoding::for_label(n.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);

    let (bytes, _, _) = encoding.encode(text);
    std::fs::write(path, &bytes)?;
    Ok(())
}

/// Detect the most likely encoding of raw bytes.
pub fn detect_encoding(bytes: &[u8]) -> &'static Encoding {
    for encoding_name in CANDIDATE_ENCODINGS {
        if let Some(encoding) = Encoding::for_label(encoding_name.as_bytes()) {
            let (_, _, had_errors) = encoding.decode(bytes);
            if !had_errors {
                return encoding;
            }
        }
    }
    encoding_rs::UTF_8
}

/// Convert text from one encoding to another.
pub fn convert_encoding(text: &str, from: &str, to: &str) -> Result<String, EncodingError> {
    let from_enc = Encoding::for_label(from.as_bytes())
        .ok_or_else(|| EncodingError::Unsupported(from.into()))?;
    let to_enc =
        Encoding::for_label(to.as_bytes()).ok_or_else(|| EncodingError::Unsupported(to.into()))?;

    let (from_bytes, _, _) = from_enc.encode(text);

    let (result, _, _) = to_enc.decode(&from_bytes);
    Ok(result.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8_read_write_roundtrip() {
        let tmp = std::env::temp_dir().join("meld_rs_test_utf8.txt");
        let text = "Hello, world! こんにちは";
        write_with_encoding(tmp.to_str().unwrap(), text, Some("UTF-8")).unwrap();
        let read = read_with_encoding(tmp.to_str().unwrap()).unwrap();
        assert_eq!(read, text);
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_detect_encoding_utf8() {
        let bytes = b"Hello UTF-8";
        let enc = detect_encoding(bytes);
        assert_eq!(enc.name(), "UTF-8");
    }

    #[test]
    fn test_convert_encoding() {
        let result = convert_encoding("hello", "UTF-8", "UTF-8").unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_read_text_with_encoding_utf8() {
        let tmp = std::env::temp_dir().join("meld_rs_test_enc_utf8.txt");
        let text = "Hello, world! こんにちは";
        std::fs::write(&tmp, text.as_bytes()).unwrap();

        let (got, enc) = read_text_with_encoding(tmp.to_str().unwrap(), None).unwrap();
        assert_eq!(got, text);
        assert_eq!(enc, "UTF-8");

        // A preferred encoding is preserved verbatim when it decodes cleanly.
        let (got2, enc2) = read_text_with_encoding(tmp.to_str().unwrap(), Some("UTF-8")).unwrap();
        assert_eq!(got2, text);
        assert_eq!(enc2, "UTF-8");

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_read_text_with_encoding_latin1() {
        let tmp = std::env::temp_dir().join("meld_rs_test_enc_latin1.txt");
        // "café" as Latin-1 (0xE9 is not valid UTF-8, so detection must fall back).
        std::fs::write(&tmp, b"caf\xe9").unwrap();

        let (text, enc) = read_text_with_encoding(tmp.to_str().unwrap(), None).unwrap();
        assert_eq!(text, "café");
        assert_ne!(enc, "UTF-8");

        std::fs::remove_file(&tmp).ok();
    }
}
