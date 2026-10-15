//! Iniezione del tracciamento della directory corrente (OSC 7) nella shell remota.

use crate::shell::Cmd;
use crate::{Shell, SshError};

// Le sequenze \033 e \007 restano letterali: le interpreta printf sul server.
// La riga inizia con uno spazio, così con HISTCONTROL=ignorespace non finisce nella history.
const BASH: &str = r#" PROMPT_COMMAND='printf "\033]7;file://%s%s\007" "$HOSTNAME" "$PWD"';clear"#;
const TCSH: &str = r#" alias precmd 'printf "\033]7;file://%s%s\007" "$HOST" "$cwd"';clear"#;

/// Riga di comando che attiva OSC 7 per la shell indicata.
pub fn injection_line(shell: &str) -> Result<String, SshError> {
    let body = match shell {
        "bash" => BASH,
        "tcsh" | "csh" => TCSH,
        _ => return Err(SshError::Connect(format!("shell non supportata: {shell}"))),
    };
    Ok(format!("{body}\n"))
}

impl Shell {
    pub async fn inject_cwd_tracking(&self, shell: &str) -> Result<(), SshError> {
        let line = injection_line(shell)?;
        self.tx
            .send(Cmd::Write(line.into_bytes()))
            .await
            .map_err(|_| SshError::ChannelClosed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_line_is_literal_and_newline_terminated() {
        let l = injection_line("bash").unwrap();
        assert!(l.starts_with(' '));
        assert!(l.ends_with(";clear\n"));
        assert!(l.contains(r"\033]7;file://"));
        assert!(!l.contains('\x1b'));
    }

    #[test]
    fn unknown_shell_is_rejected() {
        assert!(injection_line("fish").is_err());
    }
}
