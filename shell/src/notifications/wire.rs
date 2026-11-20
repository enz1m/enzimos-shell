use async_trait::async_trait;
use enzui_shell::i18n::tr;
use std::{
    io,
    os::fd::{BorrowedFd, OwnedFd},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::{
        UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
    time::{Instant, timeout_at},
};
use zbus::{
    Message,
    connection::socket::{ReadHalf, Socket, Split, WriteHalf},
    fdo::ConnectionCredentials,
};

pub const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_PROTOCOL_BYTES: usize = 128 * 1024 * 1024;
const FRAME_TIMEOUT: Duration = Duration::from_secs(3);
#[derive(Debug)]
pub struct BoundedSocket {
    stream: UnixStream,
    dropped: Arc<AtomicU64>,
}
impl BoundedSocket {
    pub fn new(stream: UnixStream, dropped: Arc<AtomicU64>) -> Self {
        Self { stream, dropped }
    }
}
impl Socket for BoundedSocket {
    type ReadHalf = BoundedRead<OwnedReadHalf>;
    type WriteHalf = BoundedWrite<OwnedWriteHalf>;
    fn split(self) -> Split<Self::ReadHalf, Self::WriteHalf> {
        let (read, write) = self.stream.into_split();
        Split::new(
            BoundedRead {
                inner: read,
                dropped: self.dropped,
            },
            BoundedWrite(write),
        )
    }
}
#[derive(Debug)]
pub struct BoundedRead<R> {
    inner: R,
    dropped: Arc<AtomicU64>,
}
#[derive(Debug)]
pub struct BoundedWrite<W>(W);
struct WriteGuard(Option<OwnedFd>);
impl Drop for WriteGuard {
    fn drop(&mut self) {
        if let Some(fd) = &self.0 {
            let _ = rustix::net::shutdown(fd, rustix::net::Shutdown::Both);
        }
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        tr("shell-invalid-notification-bus-frame"),
    )
}
fn frame_size(header: &[u8]) -> io::Result<usize> {
    if header.len() != 16 || header[3] != 1 {
        return Err(invalid());
    }
    let word = |at: usize| -> io::Result<u32> {
        let bytes = header[at..at + 4].try_into().map_err(|_| invalid())?;
        match header[0] {
            b'l' => Ok(u32::from_le_bytes(bytes)),
            b'B' => Ok(u32::from_be_bytes(bytes)),
            _ => Err(invalid()),
        }
    };
    let header_bytes = 16u64 + u64::from(word(12)?);
    let total = ((header_bytes + 7) & !7) + u64::from(word(4)?);
    if total > MAX_PROTOCOL_BYTES as u64 {
        return Err(invalid());
    }
    usize::try_from(total).map_err(|_| invalid())
}
fn timed_out(_: tokio::time::error::Elapsed) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        tr("shell-notification-bus-frame-timed-out"),
    )
}
#[async_trait]
impl<R: ReadHalf> ReadHalf for BoundedRead<R> {
    async fn recvmsg(&mut self, buffer: &mut [u8]) -> io::Result<(usize, Vec<OwnedFd>)> {
        let (count, fds) = self.inner.recvmsg(buffer).await?;
        if !fds.is_empty() {
            return Err(invalid());
        }
        Ok((count, fds))
    }
    async fn peer_credentials(&mut self) -> io::Result<ConnectionCredentials> {
        self.inner.peer_credentials().await
    }
    async fn receive_message(
        &mut self,
        sequence: u64,
        prefetched: &mut Vec<u8>,
        fds: &mut Vec<OwnedFd>,
    ) -> zbus::Result<Message> {
        if !fds.is_empty() {
            fds.clear();
            return Err(invalid().into());
        }
        loop {
            if prefetched.is_empty() {
                let mut first = [0; 1];
                let (count, _) = self.recvmsg(&mut first).await?;
                if count == 0 {
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
                }
                prefetched.push(first[0]);
            }
            let deadline = Instant::now() + FRAME_TIMEOUT;
            let mut header = [0; 16];
            while prefetched.len() < 16 {
                let needed = 16 - prefetched.len();
                let (count, _) = timeout_at(deadline, self.recvmsg(&mut header[..needed]))
                    .await
                    .map_err(timed_out)??;
                if count == 0 {
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
                }
                prefetched.extend_from_slice(&header[..count]);
            }
            let length = frame_size(&prefetched[..16])?;
            if length <= MAX_MESSAGE_BYTES {
                let message = timeout_at(
                    deadline,
                    self.inner.receive_message(sequence, prefetched, fds),
                )
                .await
                .map_err(timed_out)??;
                if !message.data().fds().is_empty() {
                    return Err(invalid().into());
                }
                return Ok(message);
            }
            let buffered = prefetched.len().min(length);
            prefetched.drain(..buffered);
            let mut remaining = length - buffered;
            let mut scratch = [0u8; 16384];
            while remaining > 0 {
                let next = remaining.min(scratch.len());
                let (count, _) = timeout_at(deadline, self.recvmsg(&mut scratch[..next]))
                    .await
                    .map_err(timed_out)??;
                if count == 0 {
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
                }
                remaining -= count;
                tokio::task::yield_now().await;
            }
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
#[async_trait]
impl<W: WriteHalf + AsRef<UnixStream>> WriteHalf for BoundedWrite<W> {
    async fn send_message(&mut self, message: &Message) -> zbus::Result<()> {
        if message.data().len() > MAX_MESSAGE_BYTES || !message.data().fds().is_empty() {
            return Err(zbus::Error::ExcessData);
        }
        let mut guard = WriteGuard(Some(
            rustix::io::dup(self.0.as_ref()).map_err(io::Error::from)?,
        ));
        let data = message.data();
        let mut position = 0;
        while position < data.len() {
            let count = self.0.sendmsg(&data[position..], &[]).await?;
            if count == 0 {
                return Err(io::Error::from(io::ErrorKind::WriteZero).into());
            }
            position += count;
        }
        guard.0 = None;
        Ok(())
    }
    async fn sendmsg(&mut self, buffer: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<usize> {
        if !fds.is_empty() {
            return Err(invalid());
        }
        self.0.sendmsg(buffer, fds).await
    }
    async fn close(&mut self) -> io::Result<()> {
        self.0.close().await
    }
    async fn peer_credentials(&mut self) -> io::Result<ConnectionCredentials> {
        self.0.peer_credentials().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, sync::atomic::AtomicUsize};
    #[derive(Debug)]
    struct Input {
        bytes: io::Cursor<Vec<u8>>,
        largest_read: Arc<AtomicUsize>,
        partial: usize,
    }
    #[async_trait]
    impl ReadHalf for Input {
        async fn recvmsg(&mut self, buffer: &mut [u8]) -> io::Result<(usize, Vec<OwnedFd>)> {
            self.largest_read.fetch_max(buffer.len(), Ordering::Relaxed);
            let length = buffer.len().min(self.partial);
            Ok((self.bytes.read(&mut buffer[..length])?, vec![]))
        }
    }
    fn reader(bytes: Vec<u8>, partial: usize) -> BoundedRead<Input> {
        BoundedRead {
            inner: Input {
                bytes: io::Cursor::new(bytes),
                largest_read: Arc::new(AtomicUsize::new(0)),
                partial,
            },
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }
    fn frame(body: &[u8]) -> Vec<u8> {
        Message::method_call("/org/freedesktop/Notifications", "Notify")
            .unwrap()
            .build(&body)
            .unwrap()
            .data()
            .bytes()
            .to_vec()
    }
    #[tokio::test]
    async fn wire_async_behaviour() {
        // Oversized frames are drained in small chunks before a valid peer frame.
        {
            let mut bytes = frame(&vec![0; MAX_MESSAGE_BYTES + 1]);
            bytes.extend(frame(b"healthy fixture"));
            let mut reader = reader(bytes, 4096);
            let message = reader
                .receive_message(1, &mut vec![], &mut vec![])
                .await
                .unwrap();
            assert_eq!(
                message.body().deserialize::<Vec<u8>>().unwrap(),
                b"healthy fixture"
            );
            assert_eq!(reader.dropped.load(Ordering::Relaxed), 1);
            assert!(reader.inner.largest_read.load(Ordering::Relaxed) <= 16384);
        }
        // Cancelling a partial write closes the socket before another frame.
        {
            use tokio::io::AsyncReadExt;
            let (client, mut peer) = UnixStream::pair().unwrap();
            let (_, write) = client.into_split();
            let mut writer = BoundedWrite(write);
            let message = Message::method_call("/fixture", "Fixture")
                .unwrap()
                .build(&vec![0u8; MAX_MESSAGE_BYTES / 2])
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(50), writer.send_message(&message))
                    .await
                    .is_err()
            );
            let next = Message::method_call("/fixture", "Fixture")
                .unwrap()
                .build(&())
                .unwrap();
            assert!(writer.send_message(&next).await.is_err());
            let mut bytes = Vec::new();
            tokio::time::timeout(Duration::from_secs(1), peer.read_to_end(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            assert!(!bytes.is_empty());
            assert!(bytes.len() < message.data().len());
        }
    }
}
