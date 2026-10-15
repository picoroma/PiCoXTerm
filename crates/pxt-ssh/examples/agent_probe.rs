//! Spike M0: verifica che russh raggiunga gli agent di Windows e legga le identità.
use russh::keys::agent::client::AgentClient;

#[tokio::main]
async fn main() {
    match AgentClient::connect_named_pipe(r"\.\pipe\openssh-ssh-agent").await {
        Ok(mut a) => match a.request_identities().await {
            Ok(ids) => println!("OpenSSH agent: OK, {} identità", ids.len()),
            Err(e) => println!("OpenSSH agent: connesso ma errore identità: {e}"),
        },
        Err(e) => println!("OpenSSH agent: non raggiungibile: {e}"),
    }
    match AgentClient::connect_pageant().await {
        Ok(mut a) => match a.request_identities().await {
            Ok(ids) => println!("Pageant: OK, {} identità", ids.len()),
            Err(e) => println!("Pageant: connesso ma errore identità: {e}"),
        },
        Err(e) => println!("Pageant: non raggiungibile: {e}"),
    }
}
