//! Modifica di file di testo remoti (D-19, sezione 7.6): lettura e scrittura **in memoria**, con limite
//! di dimensione. È l'unica eccezione, motivata e limitata, alla regola "mai un file intero in memoria":
//! vale solo qui e solo sotto `MAX_EDIT_BYTES`; i trasferimenti restano sempre in streaming.

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{EntryKind, FsError, RemoteFs, Result};

/// Dimensione massima di un file apribile nell'editor.
pub const MAX_EDIT_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    /// ISO-8859-1: ripiego per i file che non sono UTF-8 valido. Ogni byte è un carattere, quindi
    /// rileggere e riscrivere non altera nulla.
    Latin1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Eol {
    Lf,
    Crlf,
}

/// Un file di testo letto per la modifica. `content` usa sempre `\n` come fine riga: la conversione
/// verso `eol` si fa al salvataggio.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFile {
    pub content: String,
    pub encoding: Encoding,
    pub eol: Eol,
    /// Il file mescolava fine riga diversi: salvando verranno uniformati a `eol`.
    pub mixed_eol: bool,
    pub size: u64,
    pub modified: Option<i64>,
}

/// Stato del file al momento dell'apertura, per accorgersi di modifiche concorrenti.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stamp {
    pub size: u64,
    pub modified: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SaveOutcome {
    Saved {
        size: u64,
        modified: Option<i64>,
    },
    /// Il file sul server è cambiato (o è sparito) dopo l'apertura: non è stato scritto nulla.
    Conflict {
        size: Option<u64>,
        modified: Option<i64>,
        missing: bool,
    },
}

/// Decodifica i byte di un file di testo. Rifiuta i file binari (presenza di byte NUL).
pub fn decode(bytes: &[u8]) -> Result<(String, Encoding, Eol, bool)> {
    if bytes.contains(&0) {
        return Err(FsError::NotText);
    }
    let (body, bom) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => (rest, true),
        None => (bytes, false),
    };
    let (text, encoding) = match std::str::from_utf8(body) {
        Ok(s) => (
            s.to_string(),
            if bom {
                Encoding::Utf8Bom
            } else {
                Encoding::Utf8
            },
        ),
        Err(_) => (body.iter().map(|&b| b as char).collect(), Encoding::Latin1),
    };
    let crlf = text.matches("\r\n").count();
    let lf_total = text.matches('\n').count();
    let lone_lf = lf_total - crlf;
    let eol = if crlf > lone_lf { Eol::Crlf } else { Eol::Lf };
    let mixed = crlf > 0 && lone_lf > 0;
    Ok((text.replace("\r\n", "\n"), encoding, eol, mixed))
}

/// Operazione inversa di `decode`: riapplica fine riga e codifica originali.
pub fn encode(content: &str, encoding: Encoding, eol: Eol) -> Result<Vec<u8>> {
    // Il testo dell'editor non dovrebbe contenere `\r`; se c'è, lo si lascia com'è.
    let text = match eol {
        Eol::Lf => content.to_string(),
        Eol::Crlf => content.replace('\n', "\r\n"),
    };
    Ok(match encoding {
        Encoding::Utf8 => text.into_bytes(),
        Encoding::Utf8Bom => {
            let mut v = vec![0xEF, 0xBB, 0xBF];
            v.extend_from_slice(text.as_bytes());
            v
        }
        Encoding::Latin1 => text
            .chars()
            .map(|c| u8::try_from(u32::from(c)).map_err(|_| FsError::Unencodable(c)))
            .collect::<Result<Vec<u8>>>()?,
    })
}

/// Legge un file di testo per la modifica.
pub async fn read_text<F: RemoteFs>(fs: &F, path: &str, max: u64) -> Result<TextFile> {
    let meta = fs.stat(path).await?;
    if meta.kind != EntryKind::File {
        return Err(FsError::NotRegularFile(path.to_string()));
    }
    if meta.size > max {
        return Err(FsError::TooLarge {
            size: meta.size,
            max,
        });
    }
    let mut reader = fs.open_read(path).await?;
    let mut bytes = Vec::with_capacity(usize::try_from(meta.size).unwrap_or(0));
    // `take(max + 1)`: se il file è cresciuto dopo lo stat, ci si accorge del superamento.
    (&mut reader)
        .take(max + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| FsError::Sftp(e.to_string()))?;
    if bytes.len() as u64 > max {
        return Err(FsError::TooLarge {
            size: bytes.len() as u64,
            max,
        });
    }
    let (content, encoding, eol, mixed_eol) = decode(&bytes)?;
    Ok(TextFile {
        content,
        encoding,
        eol,
        mixed_eol,
        size: bytes.len() as u64,
        modified: meta.modified,
    })
}

/// Salva il contenuto sul file remoto. Se `base` è presente e il file è cambiato dopo l'apertura
/// (data o dimensione diverse, oppure sparito) non scrive nulla e restituisce `Conflict`.
///
/// La scrittura avviene **sul posto** (il file viene troncato e riscritto), non tramite file
/// temporaneo e rinomina: così restano invariati permessi, proprietario, link fisici e simbolici.
/// Il contenuto è già interamente in memoria e sotto il limite, quindi il rischio è una scrittura
/// interrotta a metà da un errore di rete.
pub async fn write_text<F: RemoteFs>(
    fs: &F,
    path: &str,
    content: &str,
    encoding: Encoding,
    eol: Eol,
    base: Option<Stamp>,
    max: u64,
) -> Result<SaveOutcome> {
    let bytes = encode(content, encoding, eol)?;
    if bytes.len() as u64 > max {
        return Err(FsError::TooLarge {
            size: bytes.len() as u64,
            max,
        });
    }
    if let Some(base) = base {
        match fs.stat(path).await {
            Ok(now) => {
                if now.size != base.size || now.modified != base.modified {
                    return Ok(SaveOutcome::Conflict {
                        size: Some(now.size),
                        modified: now.modified,
                        missing: false,
                    });
                }
            }
            Err(FsError::NotFound(_)) => {
                return Ok(SaveOutcome::Conflict {
                    size: None,
                    modified: None,
                    missing: true,
                })
            }
            Err(e) => return Err(e),
        }
    }
    let mut writer = fs.open_write(path).await?;
    writer
        .write_all(&bytes)
        .await
        .map_err(|e| FsError::Sftp(e.to_string()))?;
    writer
        .shutdown()
        .await
        .map_err(|e| FsError::Sftp(e.to_string()))?;
    let after = fs.stat(path).await?;
    Ok(SaveOutcome::Saved {
        size: after.size,
        modified: after.modified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_lf_roundtrip() {
        let (text, enc, eol, mixed) = decode("ciao è\nmondo\n".as_bytes()).unwrap();
        assert_eq!((enc, eol, mixed), (Encoding::Utf8, Eol::Lf, false));
        assert_eq!(
            encode(&text, enc, eol).unwrap(),
            "ciao è\nmondo\n".as_bytes()
        );
    }

    #[test]
    fn crlf_is_normalized_and_restored() {
        let src = b"a\r\nb\r\nc";
        let (text, enc, eol, mixed) = decode(src).unwrap();
        assert_eq!(text, "a\nb\nc");
        assert_eq!((enc, eol, mixed), (Encoding::Utf8, Eol::Crlf, false));
        assert_eq!(encode(&text, enc, eol).unwrap(), src);
    }

    #[test]
    fn mixed_line_endings_are_flagged() {
        let (_, _, eol, mixed) = decode(b"a\r\nb\r\nc\nd").unwrap();
        assert_eq!(eol, Eol::Crlf);
        assert!(mixed);
    }

    #[test]
    fn bom_is_kept() {
        let src = [&[0xEF, 0xBB, 0xBF][..], "ok\n".as_bytes()].concat();
        let (text, enc, eol, _) = decode(&src).unwrap();
        assert_eq!((text.as_str(), enc), ("ok\n", Encoding::Utf8Bom));
        assert_eq!(encode(&text, enc, eol).unwrap(), src);
    }

    #[test]
    fn invalid_utf8_falls_back_to_latin1_losslessly() {
        let src = [b'c', b'a', b'f', 0xE9, b'\n'];
        let (text, enc, eol, _) = decode(&src).unwrap();
        assert_eq!(enc, Encoding::Latin1);
        assert_eq!(text, "café\n");
        assert_eq!(encode(&text, enc, eol).unwrap(), src);
    }

    #[test]
    fn binary_is_rejected() {
        assert!(matches!(decode(b"ab\0cd"), Err(FsError::NotText)));
    }

    #[test]
    fn latin1_refuses_characters_it_cannot_hold() {
        assert!(matches!(
            encode("€", Encoding::Latin1, Eol::Lf),
            Err(FsError::Unencodable('€'))
        ));
    }
}
