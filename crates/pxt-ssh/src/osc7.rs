//! Intercettazione di OSC 7 (`ESC ] 7 ; file://host/path BEL|ST`) nel flusso del terminale.
//! Il flusso non viene modificato: lo scanner si limita a osservarlo, anche a cavallo di più chunk.

const MAX_OSC_LEN: usize = 4096;

#[derive(Default)]
pub struct Osc7Scanner {
    state: State,
    buf: Vec<u8>,
}

#[derive(Default, PartialEq)]
enum State {
    #[default]
    Ground,
    Esc,
    /// Dopo `ESC ]`: si accumula il payload.
    Osc,
    /// Dentro l'OSC è arrivato un ESC, atteso `\` (ST).
    OscEsc,
}

impl Osc7Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Analizza un chunk e restituisce i percorsi comunicati con OSC 7, nell'ordine.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut found = Vec::new();
        for &b in chunk {
            match self.state {
                State::Ground => {
                    if b == 0x1b {
                        self.state = State::Esc;
                    }
                }
                State::Esc => {
                    if b == b']' {
                        self.buf.clear();
                        self.state = State::Osc;
                    } else if b != 0x1b {
                        self.state = State::Ground;
                    }
                }
                State::Osc => match b {
                    0x07 => self.finish(&mut found),
                    0x1b => self.state = State::OscEsc,
                    _ => {
                        self.buf.push(b);
                        if self.buf.len() > MAX_OSC_LEN {
                            self.reset();
                        }
                    }
                },
                State::OscEsc => {
                    if b == b'\\' {
                        self.finish(&mut found);
                    } else {
                        self.reset();
                    }
                }
            }
        }
        found
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.state = State::Ground;
    }

    fn finish(&mut self, found: &mut Vec<String>) {
        if let Some(path) = parse_payload(&self.buf) {
            found.push(path);
        }
        self.reset();
    }
}

/// `7;file://host/percent%20encoded/path` -> percorso decodificato.
fn parse_payload(payload: &[u8]) -> Option<String> {
    let rest = payload.strip_prefix(b"7;file://")?;
    let slash = rest.iter().position(|&c| c == b'/')?;
    percent_decode(&rest[slash..])
}

fn percent_decode(s: &[u8]) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' {
            let hex = std::str::from_utf8(s.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bel_terminated() {
        let mut s = Osc7Scanner::new();
        let r = s.feed(b"prompt\x1b]7;file://host/usr/sap/BGP\x07$ ");
        assert_eq!(r, vec!["/usr/sap/BGP"]);
    }

    #[test]
    fn st_terminated_and_percent_decoding() {
        let mut s = Osc7Scanner::new();
        let r = s.feed(b"\x1b]7;file://h/tmp/a%20b%C3%A8\x1b\\");
        assert_eq!(r, vec!["/tmp/a bè"]);
    }

    #[test]
    fn split_across_chunks() {
        let mut s = Osc7Scanner::new();
        assert!(s.feed(b"x\x1b]7;file://h/us").is_empty());
        assert!(s.feed(b"r/sap/SYS").is_empty());
        assert_eq!(s.feed(b"\x07"), vec!["/usr/sap/SYS"]);
    }

    #[test]
    fn ignores_other_osc_and_overlong() {
        let mut s = Osc7Scanner::new();
        assert!(s.feed(b"\x1b]0;titolo\x07").is_empty());
        let mut long = b"\x1b]7;file://h/".to_vec();
        long.extend(std::iter::repeat_n(b'a', MAX_OSC_LEN + 10));
        long.push(0x07);
        assert!(s.feed(&long).is_empty());
        assert_eq!(s.feed(b"\x1b]7;file://h/ok\x07"), vec!["/ok"]);
    }

    #[test]
    fn multiple_in_one_chunk() {
        let mut s = Osc7Scanner::new();
        let r = s.feed(b"\x1b]7;file://h/a\x07xx\x1b]7;file://h/b\x07");
        assert_eq!(r, vec!["/a", "/b"]);
    }
}
