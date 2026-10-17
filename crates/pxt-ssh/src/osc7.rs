//! Intercettazione di due sequenze OSC nel flusso del terminale: OSC 7 (`ESC ] 7 ; file://host/path
//! BEL|ST`, la directory corrente) e OSC 7770 (`ESC ] 7770 ; nome BEL|ST`, privata: la risposta della
//! sonda che chiede alla shell remota *quale* shell è). Il flusso non viene modificato: lo scanner si
//! limita a osservarlo, anche a cavallo di più chunk.

const MAX_OSC_LEN: usize = 4096;

/// Ciò che lo scanner riconosce.
#[derive(Debug, PartialEq, Eq)]
pub enum OscEvent {
    /// Directory corrente (OSC 7).
    Cwd(String),
    /// Nome della shell in esecuzione, dalla sonda (OSC 7770).
    Shell(String),
}

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

    /// Analizza un chunk e restituisce gli eventi riconosciuti, nell'ordine.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<OscEvent> {
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

    fn finish(&mut self, found: &mut Vec<OscEvent>) {
        if let Some(ev) = parse_payload(&self.buf) {
            found.push(ev);
        }
        self.reset();
    }
}

/// `7;file://host/percent%20encoded/path` -> percorso decodificato; `7770;nome` -> nome della shell.
fn parse_payload(payload: &[u8]) -> Option<OscEvent> {
    if let Some(name) = payload.strip_prefix(b"7770;") {
        let name = std::str::from_utf8(name).ok()?.trim();
        // Il nome arriva da un comando eseguito sul server: si accettano solo caratteri da nome di
        // programma o percorso, mai sequenze di controllo.
        let ok = !name.is_empty()
            && name.len() <= 256
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/-_.+".contains(c));
        return ok.then(|| OscEvent::Shell(name.to_string()));
    }
    let rest = payload.strip_prefix(b"7;file://")?;
    let slash = rest.iter().position(|&c| c == b'/')?;
    percent_decode(&rest[slash..]).map(OscEvent::Cwd)
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
        assert_eq!(r, vec![OscEvent::Cwd("/usr/sap/BGP".into())]);
    }

    #[test]
    fn st_terminated_and_percent_decoding() {
        let mut s = Osc7Scanner::new();
        let r = s.feed(b"\x1b]7;file://h/tmp/a%20b%C3%A8\x1b\\");
        assert_eq!(r, vec![OscEvent::Cwd("/tmp/a bè".into())]);
    }

    #[test]
    fn split_across_chunks() {
        let mut s = Osc7Scanner::new();
        assert!(s.feed(b"x\x1b]7;file://h/us").is_empty());
        assert!(s.feed(b"r/sap/SYS").is_empty());
        assert_eq!(s.feed(b"\x07"), vec![OscEvent::Cwd("/usr/sap/SYS".into())]);
    }

    #[test]
    fn ignores_other_osc_and_overlong() {
        let mut s = Osc7Scanner::new();
        assert!(s.feed(b"\x1b]0;titolo\x07").is_empty());
        let mut long = b"\x1b]7;file://h/".to_vec();
        long.extend(std::iter::repeat_n(b'a', MAX_OSC_LEN + 10));
        long.push(0x07);
        assert!(s.feed(&long).is_empty());
        assert_eq!(
            s.feed(b"\x1b]7;file://h/ok\x07"),
            vec![OscEvent::Cwd("/ok".into())]
        );
    }

    #[test]
    fn multiple_in_one_chunk() {
        let mut s = Osc7Scanner::new();
        let r = s.feed(b"\x1b]7;file://h/a\x07xx\x1b]7;file://h/b\x07");
        assert_eq!(
            r,
            vec![OscEvent::Cwd("/a".into()), OscEvent::Cwd("/b".into())]
        );
    }

    #[test]
    fn shell_probe_reply_is_recognised_and_sanitised() {
        let mut s = Osc7Scanner::new();
        assert_eq!(
            s.feed(b"\x1b]7770;-tcsh\x07"),
            vec![OscEvent::Shell("-tcsh".into())]
        );
        assert_eq!(
            s.feed(b"\x1b]7770;/usr/bin/bash\x1b\\"),
            vec![OscEvent::Shell("/usr/bin/bash".into())]
        );
        // Vuoto o con caratteri strani: ignorato.
        assert!(s.feed(b"\x1b]7770;\x07").is_empty());
        assert!(s.feed(b"\x1b]7770;a;rm -rf\x07").is_empty());
    }
}
