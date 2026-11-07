//! Bounded, cancellable nonblocking clipboard pipes; a hostile selection owner cannot block input.
use anyhow::{Result, ensure};
use std::{
    io::{Read, Write},
    os::fd::OwnedFd,
};
use tokio::io::unix::AsyncFd;

fn prepare(fd: OwnedFd) -> Result<AsyncFd<std::fs::File>> {
    let flags = rustix::fs::fcntl_getfl(&fd)?;
    rustix::fs::fcntl_setfl(&fd, flags | rustix::fs::OFlags::NONBLOCK)?;
    // SAFETY: AsyncFd owns this File exclusively; its descriptor cannot be swapped or closed
    // while registered, and the inner File is exposed only through shared read/write access.
    Ok(unsafe { AsyncFd::register(std::fs::File::from(fd)) }?)
}
pub async fn read(fd: OwnedFd, limit: usize) -> Result<Vec<u8>> {
    let pipe = prepare(fd)?;
    let mut bytes = Vec::new();
    let mut buffer = [0; 16384];
    loop {
        let mut guard = pipe.readable().await?;
        match guard.try_io(|inner| inner.get_ref().read(&mut buffer)) {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                ensure!(bytes.len() + n <= limit, "clipboard exceeds size limit");
                bytes.extend_from_slice(&buffer[..n]);
            }
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => continue,
        }
    }
    Ok(bytes)
}
pub async fn write(fd: OwnedFd, bytes: &[u8]) -> Result<()> {
    let pipe = prepare(fd)?;
    let mut offset = 0;
    while offset < bytes.len() {
        let mut guard = pipe.writable().await?;
        match guard.try_io(|inner| inner.get_ref().write(&bytes[offset..])) {
            Ok(Ok(0)) => anyhow::bail!("clipboard pipe closed"),
            Ok(Ok(n)) => offset += n,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => continue,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;
    #[tokio::test]
    async fn clipboard_pipe_preserves_bytes() {
        let (a, b) = UnixStream::pair().unwrap();
        let data = vec![72; 200_000];
        let expected = data.clone();
        let task = tokio::spawn(async move { write(a.into(), &data).await.unwrap() });
        let got = read(b.into(), 300_000).await.unwrap();
        task.await.unwrap();
        assert_eq!(got, expected);
    }
    #[tokio::test]
    async fn pipe_limit_and_stalled_owner_are_bounded() {
        let (a, b) = UnixStream::pair().unwrap();
        let task = tokio::spawn(async move { write(a.into(), &[1; 100]).await.unwrap() });
        assert!(read(b.into(), 10).await.is_err());
        task.await.unwrap();
        let (_a, b) = UnixStream::pair().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), read(b.into(), 100))
                .await
                .is_err()
        );
    }
}
