//! Appunti condivisi (CLIPRDR), solo testo: il testo copiato sul server arriva all'app, quello copiato
//! sul computer può essere incollato nel server. File, immagini e formati vari non si trasferiscono.
//!
//! Il canale `cliprdr` chiama il "backend" quando succede qualcosa; il backend non può scrivere sulla
//! rete da solo, quindi gira gli eventi al ciclo della sessione (`session::run`), che risponde.

use ironrdp::cliprdr::backend::CliprdrBackend;
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp::core::AsAny;
use tokio::sync::mpsc;

/// Oltre questa dimensione il testo non si trasferisce (in nessuna direzione).
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum ClipEvent {
    /// Il canale è pronto.
    Ready,
    /// Il server chiede i formati disponibili sul nostro lato (all'avvio del canale).
    RequestFormatList,
    /// Sul server è stato copiato qualcosa, in questi formati.
    RemoteCopy(Vec<ClipboardFormatId>),
    /// Il server vuole incollare dal nostro lato, in questo formato.
    DataRequest(ClipboardFormatId),
    /// Dati arrivati dal server (`None` = il server non ha potuto darli).
    DataResponse(Option<Vec<u8>>),
}

#[derive(Debug)]
pub struct TextBackend {
    tx: mpsc::UnboundedSender<ClipEvent>,
    temp_dir: String,
}

impl TextBackend {
    pub fn new(tx: mpsc::UnboundedSender<ClipEvent>) -> Self {
        Self {
            tx,
            temp_dir: std::env::temp_dir().to_string_lossy().into_owned(),
        }
    }

    fn emit(&self, e: ClipEvent) {
        let _ = self.tx.send(e);
    }
}

impl AsAny for TextBackend {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl CliprdrBackend for TextBackend {
    fn temporary_directory(&self) -> &str {
        &self.temp_dir
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        // Niente file né blocchi: solo testo, con i nomi di formato lunghi.
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
    }

    fn on_ready(&mut self) {
        self.emit(ClipEvent::Ready);
    }

    fn on_request_format_list(&mut self) {
        self.emit(ClipEvent::RequestFormatList);
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.emit(ClipEvent::RemoteCopy(
            available_formats.iter().map(|f| f.id).collect(),
        ));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.emit(ClipEvent::DataRequest(request.format));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        self.emit(ClipEvent::DataResponse(if response.is_error() {
            None
        } else {
            Some(response.data().to_vec())
        }));
    }

    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, _: LockDataId) {}
}

/// Testo in UTF-16 little-endian con terminatore, e con gli a capo `\r\n` che il formato richiede.
pub fn encode_text(text: &str) -> Vec<u8> {
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let mut out: Vec<u8> = crlf.encode_utf16().flat_map(u16::to_le_bytes).collect();
    out.extend_from_slice(&[0, 0]);
    out
}

/// Il contrario: toglie il terminatore e riporta gli a capo a `\n`.
pub fn decode_text(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units).replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrips_through_the_wire_format() {
        let s = "ciao\nperché è così — ✓\nfine";
        let wire = encode_text(s);
        assert!(wire.ends_with(&[0, 0]));
        assert_eq!(decode_text(&wire), s);
    }

    #[test]
    fn line_endings_are_crlf_on_the_wire_and_lf_locally() {
        let wire = encode_text("a\nb\r\nc");
        let units: Vec<u16> = wire
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let text = String::from_utf16(&units[..units.len() - 1]).unwrap();
        assert_eq!(text, "a\r\nb\r\nc");
        assert_eq!(decode_text(&wire), "a\nb\nc");
    }

    #[test]
    fn a_truncated_or_unterminated_buffer_does_not_panic() {
        assert_eq!(decode_text(&[]), "");
        assert_eq!(decode_text(b"a"), "");
        assert_eq!(decode_text(&[b'a', 0, b'b', 0]), "ab");
    }
}
