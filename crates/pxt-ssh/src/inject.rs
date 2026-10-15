//! Iniezione del tracciamento della directory corrente (OSC 7) nella shell remota.

use std::time::Duration;

use russh::ChannelMsg;

use crate::shell::Cmd;
use crate::{Shell, SshError};

// Le sequenze \033 e \007 restano letterali: le interpreta printf sul server.
// Ogni riga inizia con uno spazio, così con HISTCONTROL=ignorespace non finisce nella history,
// e termina con `clear` per non lasciare la riga di servizio sullo schermo.
const BASH: &str = r#" PROMPT_COMMAND='printf "\033]7;file://%s%s\007" "$HOSTNAME" "$PWD"';clear"#;
const ZSH: &str = r#" __pxt_cwd() { printf "\033]7;file://%s%s\007" "$HOST" "$PWD"; }; precmd_functions+=(__pxt_cwd); clear"#;
const TCSH: &str = r#" alias precmd 'printf "\033]7;file://%s%s\007" "$HOST" "$cwd"';clear"#;

/// Famiglia di shell supportata dal tracciamento, a partire dal nome (`bash`, `/bin/zsh`, `tcsh`…).
pub fn shell_family(name: &str) -> Option<&'static str> {
    let base = name.trim().rsplit('/').next().unwrap_or("");
    match base {
        "bash" => Some("bash"),
        "zsh" => Some("zsh"),
        "tcsh" => Some("tcsh"),
        _ => None,
    }
}

/// Riga di comando che attiva OSC 7 per la shell indicata.
pub fn injection_line(shell: &str) -> Result<String, SshError> {
    let body = match shell_family(shell) {
        Some("bash") => BASH,
        Some("zsh") => ZSH,
        Some("tcsh") => TCSH,
        _ => return Err(SshError::UnsupportedShell(shell.trim().to_string())),
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

    /// Nome della shell di login dell'utente (`$SHELL`), letto con un canale exec separato.
    pub async fn detect_shell(&self) -> Result<String, SshError> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(true, "echo $SHELL").await?;
        let mut out = Vec::new();
        let read = async {
            while let Some(msg) = channel.wait().await {
                match msg {
                    ChannelMsg::Data { data } => out.extend_from_slice(&data),
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(4), read)
            .await
            .map_err(|_| SshError::Timeout(4))?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// Rileva la shell e attiva il tracciamento. Restituisce il nome della shell.
    pub async fn enable_cwd_tracking(&self) -> Result<String, SshError> {
        let shell = self.detect_shell().await?;
        self.inject_cwd_tracking(&shell).await?;
        Ok(shell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_literal_and_newline_terminated() {
        for sh in ["bash", "/bin/zsh", "/usr/bin/tcsh"] {
            let l = injection_line(sh).unwrap();
            assert!(l.starts_with(' '), "{sh}");
            assert!(
                l.ends_with(";clear\n") || l.ends_with("; clear\n"),
                "{sh}: {l}"
            );
            assert!(l.contains(r"\033]7;file://"), "{sh}");
            assert!(!l.contains('\x1b') && !l.contains('\x07'), "{sh}");
        }
    }

    #[test]
    fn shell_names_are_normalized_and_unknown_ones_rejected() {
        assert_eq!(shell_family("/bin/bash\n"), Some("bash"));
        assert_eq!(shell_family("zsh"), Some("zsh"));
        assert_eq!(shell_family("/bin/ash"), None);
        assert_eq!(shell_family(""), None);
        assert!(matches!(
            injection_line("/bin/fish"),
            Err(SshError::UnsupportedShell(s)) if s == "/bin/fish"
        ));
    }
}
