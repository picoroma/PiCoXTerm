//! Metodi di autenticazione: password, chiave privata (OpenSSH e .ppk), agent.

use std::path::PathBuf;
use std::sync::Arc;

use russh::client::{Handle, Handler};
use russh::keys::agent::client::AgentClient;
use russh::keys::agent::AgentIdentity;
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg};

use crate::SshError;

/// Come ci si autentica. I segreti vivono qui solo per la durata della connessione.
pub enum Credentials {
    Password(String),
    Key {
        path: PathBuf,
        passphrase: Option<String>,
    },
    Agent,
}

/// Espande un `~` iniziale nella cartella home dell'utente.
fn expand_home(path: &std::path::Path) -> PathBuf {
    let Ok(rest) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    match std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        Some(home) => PathBuf::from(home).join(rest),
        None => path.to_path_buf(),
    }
}

pub(crate) async fn authenticate<H: Handler>(
    handle: &mut Handle<H>,
    user: &str,
    creds: Credentials,
) -> Result<(), SshError>
where
    SshError: From<H::Error>,
{
    let ok = match creds {
        Credentials::Password(pw) => handle.authenticate_password(user, pw).await?.success(),
        Credentials::Key { path, passphrase } => {
            let key =
                load_secret_key(expand_home(&path), passphrase.as_deref()).map_err(
                    |e| match e {
                        russh::keys::Error::KeyIsEncrypted if passphrase.is_none() => {
                            SshError::PassphraseRequired
                        }
                        other => SshError::Key(other.to_string()),
                    },
                )?;
            let hash = handle.best_supported_rsa_hash().await?.flatten();
            handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await?
                .success()
        }
        Credentials::Agent => agent_auth(handle, user).await?,
    };
    if ok {
        Ok(())
    } else {
        Err(SshError::AuthFailed)
    }
}

/// Prova ogni identità dell'agent finché il server ne accetta una.
macro_rules! try_agent {
    ($handle:expr, $user:expr, $agent:expr) => {{
        let mut agent = $agent;
        let mut accepted = false;
        if let Ok(ids) = agent.request_identities().await {
            for id in ids {
                let AgentIdentity::PublicKey { key, .. } = id else {
                    continue;
                };
                let hash = $handle.best_supported_rsa_hash().await?.flatten();
                if let Ok(r) = $handle
                    .authenticate_publickey_with($user, key, hash, &mut agent)
                    .await
                {
                    if r.success() {
                        accepted = true;
                        break;
                    }
                }
            }
        }
        accepted
    }};
}

#[cfg(windows)]
async fn agent_auth<H: Handler>(handle: &mut Handle<H>, user: &str) -> Result<bool, SshError>
where
    SshError: From<H::Error>,
{
    let mut reachable = false;
    // Agent OpenSSH di Windows (named pipe)
    if let Ok(agent) = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        reachable = true;
        if try_agent!(handle, user, agent) {
            return Ok(true);
        }
    }
    // Pageant
    if let Ok(agent) = AgentClient::connect_pageant().await {
        reachable = true;
        if try_agent!(handle, user, agent) {
            return Ok(true);
        }
    }
    if reachable {
        Ok(false)
    } else {
        Err(SshError::AgentUnavailable)
    }
}

#[cfg(unix)]
async fn agent_auth<H: Handler>(handle: &mut Handle<H>, user: &str) -> Result<bool, SshError>
where
    SshError: From<H::Error>,
{
    let agent = AgentClient::connect_env()
        .await
        .map_err(|_| SshError::AgentUnavailable)?;
    Ok(try_agent!(handle, user, agent))
}
