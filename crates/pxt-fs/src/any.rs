//! Un filesystem remoto di tipo qualsiasi (SFTP, FTP/FTPS): serve a chi sceglie il protocollo a
//! tempo di esecuzione, come l'app, dove le sessioni sono di tipi diversi ma i comandi e il motore
//! dei trasferimenti sono gli stessi. `RemoteFs` non si può usare come oggetto (ha tipi associati e
//! metodi `async`), quindi si distribuisce con un'enum.

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::ftp::{FtpReader, FtpWriter};
use crate::{Entry, FsCapabilities, FtpFs, RemoteFs, Result, SftpFs};

pub enum AnyFs {
    Sftp(SftpFs),
    Ftp(FtpFs),
}

pub enum AnyReader {
    Sftp(<SftpFs as RemoteFs>::Reader),
    Ftp(FtpReader),
}

pub enum AnyWriter {
    Sftp(<SftpFs as RemoteFs>::Writer),
    Ftp(FtpWriter),
}

impl AsyncRead for AnyReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AnyReader::Sftp(r) => Pin::new(r).poll_read(cx, buf),
            AnyReader::Ftp(r) => Pin::new(r).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for AnyWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            AnyWriter::Sftp(w) => Pin::new(w).poll_write(cx, data),
            AnyWriter::Ftp(w) => Pin::new(w).poll_write(cx, data),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AnyWriter::Sftp(w) => Pin::new(w).poll_flush(cx),
            AnyWriter::Ftp(w) => Pin::new(w).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AnyWriter::Sftp(w) => Pin::new(w).poll_shutdown(cx),
            AnyWriter::Ftp(w) => Pin::new(w).poll_shutdown(cx),
        }
    }
}

/// Inoltra una chiamata al backend scelto.
macro_rules! forward {
    ($self:expr, $fs:ident => $call:expr) => {
        match $self {
            AnyFs::Sftp($fs) => $call,
            AnyFs::Ftp($fs) => $call,
        }
    };
}

impl RemoteFs for AnyFs {
    type Reader = AnyReader;
    type Writer = AnyWriter;

    async fn list(&self, path: &str) -> Result<Vec<Entry>> {
        forward!(self, f => f.list(path).await)
    }

    async fn stat(&self, path: &str) -> Result<Entry> {
        forward!(self, f => f.stat(path).await)
    }

    async fn lstat(&self, path: &str) -> Result<Entry> {
        forward!(self, f => f.lstat(path).await)
    }

    async fn canonicalize(&self, path: &str) -> Result<String> {
        forward!(self, f => f.canonicalize(path).await)
    }

    async fn mkdir(&self, path: &str) -> Result<()> {
        forward!(self, f => f.mkdir(path).await)
    }

    async fn remove_file(&self, path: &str) -> Result<()> {
        forward!(self, f => f.remove_file(path).await)
    }

    async fn remove_dir(&self, path: &str) -> Result<()> {
        forward!(self, f => f.remove_dir(path).await)
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        forward!(self, f => f.rename(from, to).await)
    }

    async fn set_permissions(&self, path: &str, mode: u32) -> Result<()> {
        forward!(self, f => f.set_permissions(path, mode).await)
    }

    async fn set_modified(&self, path: &str, secs: i64) -> Result<()> {
        forward!(self, f => f.set_modified(path, secs).await)
    }

    async fn open_read(&self, path: &str) -> Result<Self::Reader> {
        Ok(match self {
            AnyFs::Sftp(f) => AnyReader::Sftp(f.open_read(path).await?),
            AnyFs::Ftp(f) => AnyReader::Ftp(f.open_read(path).await?),
        })
    }

    async fn open_write(&self, path: &str) -> Result<Self::Writer> {
        Ok(match self {
            AnyFs::Sftp(f) => AnyWriter::Sftp(f.open_write(path).await?),
            AnyFs::Ftp(f) => AnyWriter::Ftp(f.open_write(path).await?),
        })
    }

    async fn open_read_at(&self, path: &str, offset: u64) -> Result<Self::Reader> {
        Ok(match self {
            AnyFs::Sftp(f) => AnyReader::Sftp(f.open_read_at(path, offset).await?),
            AnyFs::Ftp(f) => AnyReader::Ftp(f.open_read_at(path, offset).await?),
        })
    }

    async fn open_write_at(&self, path: &str, offset: u64) -> Result<Self::Writer> {
        Ok(match self {
            AnyFs::Sftp(f) => AnyWriter::Sftp(f.open_write_at(path, offset).await?),
            AnyFs::Ftp(f) => AnyWriter::Ftp(f.open_write_at(path, offset).await?),
        })
    }

    fn capabilities(&self) -> FsCapabilities {
        forward!(self, f => f.capabilities())
    }
}
