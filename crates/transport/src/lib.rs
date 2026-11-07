//! Pinned-key Noise sessions, bounded framing, keepalives and prioritized clipboard streaming.
#![forbid(unsafe_code)]

mod identity;
pub use identity::{Identity, parse_public};

use anyhow::{Context, Result, bail, ensure};
use desklink_protocol::{
    ClipboardReceiver, ClipboardTransfer, MAX_FRAME, Message, VERSION, decode, encode,
};
use identity::noise;
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::{mpsc, watch},
    time::{Instant, timeout},
};
use tokio_util::sync::CancellationToken;

pub type PeerKey = [u8; 32];
pub type EventSender = mpsc::Sender<NetworkEvent>;
const DEADLINE: Duration = Duration::from_secs(10);
static IDS: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct PeerHandle {
    pub id: u64,
    pub outbound: bool,
    control: mpsc::Sender<Message>,
    bulk: watch::Sender<Option<Arc<ClipboardTransfer>>>,
    pub cancel: CancellationToken,
}

impl PeerHandle {
    pub fn send(&self, message: Message) -> Result<()> {
        if self.control.try_send(message).is_err() {
            self.cancel.cancel();
            bail!("peer cannot keep up; closing the connection to preserve input state");
        }
        Ok(())
    }
    pub fn clipboard(&self, transfer: Arc<ClipboardTransfer>) {
        self.bulk.send_replace(Some(transfer));
    }
}

#[derive(Debug)]
pub enum NetworkEvent {
    Connected {
        key: PeerKey,
        hello: Message,
        handle: PeerHandle,
    },
    Message {
        key: PeerKey,
        id: u64,
        message: Message,
    },
    Clipboard {
        key: PeerKey,
        id: u64,
        transfer: ClipboardTransfer,
    },
    Disconnected {
        key: PeerKey,
        id: u64,
    },
}

/// Does not publish a peer until both static identities have passed the local trust policy.
pub async fn session(
    mut stream: TcpStream,
    outbound: bool,
    identity: Arc<Identity>,
    trusted: Arc<HashSet<PeerKey>>,
    hello: Message,
    events: EventSender,
    shutdown: CancellationToken,
) -> Result<()> {
    stream.set_nodelay(true)?;
    let (key, noise) = timeout(
        DEADLINE,
        handshake(&mut stream, outbound, &identity, &trusted),
    )
    .await
    .context("handshake timed out")??;
    let state = Arc::new(Mutex::new(noise));
    write_message(&mut stream, &state, &hello).await?;
    let remote = timeout(DEADLINE, read_message(&mut stream, &state))
        .await
        .context("peer hello timed out")??;
    match &remote {
        Message::Hello {
            version,
            name,
            geometry,
            ..
        } if *version == VERSION && name.len() <= 64 && geometry.valid() => {}
        _ => bail!("unsupported or invalid peer hello"),
    }
    let id = IDS.fetch_add(1, Ordering::Relaxed);
    let cancel = shutdown.child_token();
    let (control, rx) = mpsc::channel(256);
    let (bulk, bulk_rx) = watch::channel(None);
    let handle = PeerHandle {
        id,
        outbound,
        control: control.clone(),
        bulk,
        cancel: cancel.clone(),
    };
    events
        .send(NetworkEvent::Connected {
            key,
            hello: remote,
            handle,
        })
        .await?;
    let (mut reader, mut writer) = stream.into_split();
    let result = tokio::select! {
        _=cancel.cancelled()=>Ok(()),
        result=read_loop(&mut reader,&state,key,id,&events,&control)=>result,
        result=write_loop(&mut writer,&state,rx,bulk_rx)=>result,
    };
    cancel.cancel();
    let _ = events.send(NetworkEvent::Disconnected { key, id }).await;
    result
}

async fn handshake(
    stream: &mut TcpStream,
    outbound: bool,
    identity: &Identity,
    trusted: &HashSet<PeerKey>,
) -> Result<(PeerKey, snow::TransportState)> {
    let builder = noise(
        noise(snow::Builder::new(noise(identity::PATTERN.parse())?).prologue(b"desklink/1"))?
            .local_private_key(identity.private.as_ref()),
    )?;
    let mut state = noise(if outbound {
        builder.build_initiator()
    } else {
        builder.build_responder()
    })?;
    let mut buffer = vec![0; 4096];
    for turn in 0..3 {
        if (turn % 2 == 0) == outbound {
            let n = noise(state.write_message(&[], &mut buffer))?;
            write_frame(stream, &buffer[..n]).await?;
        } else {
            let data = read_frame(stream, 4096).await?;
            noise(state.read_message(&data, &mut buffer))?;
        }
        if let Some(key) = state.get_remote_static() {
            let key: PeerKey = key.try_into().context("invalid peer key")?;
            ensure!(
                trusted.contains(&key),
                "untrusted peer {}; exchange public keys with `desklink trust`",
                hex::encode(key)
            );
        }
    }
    let key: PeerKey = state
        .get_remote_static()
        .context("peer omitted its identity")?
        .try_into()?;
    ensure!(trusted.contains(&key), "untrusted peer");
    Ok((key, noise(state.into_transport_mode())?))
}

async fn read_frame<R: AsyncRead + Unpin>(read: &mut R, limit: usize) -> Result<Vec<u8>> {
    let n = read.read_u32().await? as usize;
    ensure!(n > 0 && n <= limit, "invalid frame length {n}");
    let mut bytes = vec![0; n];
    read.read_exact(&mut bytes).await?;
    Ok(bytes)
}

async fn write_frame<W: AsyncWrite + Unpin>(write: &mut W, bytes: &[u8]) -> Result<()> {
    write.write_u32(bytes.len() as u32).await?;
    write.write_all(bytes).await?;
    Ok(())
}

async fn read_message<R: AsyncRead + Unpin>(
    read: &mut R,
    state: &Mutex<snow::TransportState>,
) -> Result<Message> {
    let bytes = read_frame(read, MAX_FRAME + 16).await?;
    let mut plain = vec![0; MAX_FRAME];
    let n = noise(
        state
            .lock()
            .map_err(|_| anyhow::anyhow!("cipher lock poisoned"))?
            .read_message(&bytes, &mut plain),
    )?;
    Ok(decode(&plain[..n])?)
}

async fn write_message<W: AsyncWrite + Unpin>(
    write: &mut W,
    state: &Mutex<snow::TransportState>,
    message: &Message,
) -> Result<()> {
    let plain = encode(message)?;
    let mut cipher = vec![0; plain.len() + 16];
    let n = noise(
        state
            .lock()
            .map_err(|_| anyhow::anyhow!("cipher lock poisoned"))?
            .write_message(&plain, &mut cipher),
    )?;
    timeout(Duration::from_secs(3), write_frame(write, &cipher[..n]))
        .await
        .context("peer write stalled")??;
    Ok(())
}

async fn read_loop<R: AsyncRead + Unpin>(
    read: &mut R,
    state: &Mutex<snow::TransportState>,
    key: PeerKey,
    id: u64,
    events: &EventSender,
    control: &mpsc::Sender<Message>,
) -> Result<()> {
    let mut clipboard = ClipboardReceiver::default();
    let mut started = None;
    loop {
        let message = timeout(DEADLINE, read_message(read, state))
            .await
            .context("peer stopped responding")??;
        if started.is_some_and(|t: Instant| t.elapsed() > Duration::from_secs(20)) {
            bail!("clipboard transfer expired");
        }
        match message {
            Message::Ping(n) => control.try_send(Message::Pong(n))?,
            Message::Pong(_) => {}
            Message::Hello { .. } => bail!("unexpected hello in established session"),
            m @ (Message::ClipboardStart { .. }
            | Message::ClipboardChunk { .. }
            | Message::ClipboardEnd) => {
                if matches!(m, Message::ClipboardStart { .. }) {
                    started = Some(Instant::now());
                }
                if let Some(transfer) = clipboard.accept(m)? {
                    started = None;
                    events
                        .send(NetworkEvent::Clipboard { key, id, transfer })
                        .await?;
                }
            }
            message => {
                events
                    .send(NetworkEvent::Message { key, id, message })
                    .await?
            }
        }
    }
}

struct BulkCursor {
    transfer: Arc<ClipboardTransfer>,
    index: usize,
    offset: usize,
}
impl BulkCursor {
    fn next(&mut self) -> Option<Message> {
        if self.index == 0 {
            self.index = 1;
            return self.transfer.messages().next();
        }
        let bytes = &self.transfer.content.bytes;
        if self.offset < bytes.len() {
            let end = (self.offset + desklink_protocol::CLIPBOARD_CHUNK).min(bytes.len());
            let data = bytes[self.offset..end].to_vec();
            self.offset = end;
            return Some(Message::ClipboardChunk { data });
        }
        if self.index == 1 {
            self.index = 2;
            return Some(Message::ClipboardEnd);
        }
        None
    }
}

async fn write_loop<W: AsyncWrite + Unpin>(
    write: &mut W,
    state: &Mutex<snow::TransportState>,
    mut rx: mpsc::Receiver<Message>,
    mut bulk: watch::Receiver<Option<Arc<ClipboardTransfer>>>,
) -> Result<()> {
    let mut ping = tokio::time::interval(Duration::from_secs(2));
    let mut serial = 0;
    let mut active: Option<BulkCursor> = None;
    loop {
        let message = tokio::select! {
            biased;
            message=rx.recv()=>match message {Some(m)=>m,None=>return Ok(())},
            _=ping.tick()=>{
                serial += 1;
                Message::Ping(serial)
            },
            result=bulk.changed(),if active.is_none()=>{
                result?;
                if let Some(transfer) = bulk.borrow_and_update().clone() {
                    active = Some(BulkCursor {
                        transfer,
                        index: 0,
                        offset: 0,
                    });
                }
                continue;
            }
            _=std::future::ready(()),if active.is_some()=>{
                match active.as_mut().and_then(BulkCursor::next) {
                    Some(m) => m,
                    None => {
                        active = None;
                        continue;
                    }
                }
            }
        };
        write_message(write, state, &message).await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desklink_protocol::{ClipboardContent, ClipboardKind, Geometry, Stamp};
    use tokio::net::TcpListener;
    fn hello() -> Message {
        Message::Hello {
            version: VERSION,
            name: "test".into(),
            geometry: Geometry {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            clipboard: true,
        }
    }
    #[tokio::test]
    async fn encrypted_session_delivers_input_and_large_clipboard_both_directions() {
        let a = Arc::new(Identity::generate().unwrap());
        let b = Arc::new(Identity::generate().unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (atx, mut arx) = mpsc::channel(32);
        let (btx, mut brx) = mpsc::channel(32);
        let cancel = CancellationToken::new();
        let socket = TcpStream::connect(addr).await.unwrap();
        let (other, _) = listener.accept().await.unwrap();
        let aj = tokio::spawn(session(
            socket,
            true,
            a.clone(),
            Arc::new(HashSet::from([b.public])),
            hello(),
            atx,
            cancel.clone(),
        ));
        let bj = tokio::spawn(session(
            other,
            false,
            b,
            Arc::new(HashSet::from([a.public])),
            hello(),
            btx,
            cancel.clone(),
        ));
        let ah = match arx.recv().await.unwrap() {
            NetworkEvent::Connected { handle, .. } => handle,
            _ => panic!(),
        };
        let bh = match brx.recv().await.unwrap() {
            NetworkEvent::Connected { handle, .. } => handle,
            _ => panic!(),
        };
        ah.send(Message::Geometry(Geometry {
            x: 0,
            y: 0,
            width: 900,
            height: 700,
        }))
        .unwrap();
        assert!(matches!(
            brx.recv().await.unwrap(),
            NetworkEvent::Message {
                message: Message::Geometry(_),
                ..
            }
        ));
        for (sender, receiver) in [(&ah, &mut brx), (&bh, &mut arx)] {
            let data = ClipboardContent {
                kind: ClipboardKind::Text,
                bytes: vec![b'a'; 300_000],
            };
            sender.clipboard(Arc::new(ClipboardTransfer {
                stamp: Stamp {
                    counter: 1,
                    origin: a.public,
                },
                content: data.clone(),
            }));
            let event = timeout(Duration::from_secs(5), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            match event {
                NetworkEvent::Clipboard { transfer, .. } => assert_eq!(transfer.content, data),
                _ => panic!("unexpected event"),
            }
        }
        cancel.cancel();
        assert!(aj.await.unwrap().is_ok());
        assert!(bj.await.unwrap().is_ok());
    }
    #[tokio::test]
    async fn untrusted_peer_never_establishes_a_session() {
        let a = Arc::new(Identity::generate().unwrap());
        let b = Arc::new(Identity::generate().unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let s = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (other, _) = listener.accept().await.unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        let bj = tokio::spawn(session(
            other,
            false,
            b.clone(),
            Arc::new(HashSet::new()),
            hello(),
            tx.clone(),
            cancel.clone(),
        ));
        assert!(
            session(
                s,
                true,
                a,
                Arc::new(HashSet::from([b.public])),
                hello(),
                tx.clone(),
                cancel
            )
            .await
            .is_err()
        );
        assert!(bj.await.unwrap().is_err());
        drop(tx);
        assert!(rx.recv().await.is_none());
    }
    #[tokio::test]
    async fn rejects_oversized_frame_before_allocating() {
        let (mut a, mut b) = tokio::io::duplex(8);
        a.write_u32(u32::MAX).await.unwrap();
        assert!(read_frame(&mut b, 4096).await.is_err());
    }

    #[tokio::test]
    async fn wire_rejects_tampered_and_replayed_ciphertext() {
        for tamper in [false, true] {
            let a = Identity::generate().unwrap();
            let b = Identity::generate().unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut source = TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (mut destination, _) = listener.accept().await.unwrap();
            let at = HashSet::from([b.public]);
            let bt = HashSet::from([a.public]);
            let (sa, sb) = tokio::join!(
                handshake(&mut source, true, &a, &at),
                handshake(&mut destination, false, &b, &bt)
            );
            let (_, mut sa) = sa.unwrap();
            let (_, sb) = sb.unwrap();
            let sb = Mutex::new(sb);
            let plain = encode(&Message::Pong(42)).unwrap();
            let mut cipher = vec![0; plain.len() + 16];
            let n = sa.write_message(&plain, &mut cipher).unwrap();
            cipher.truncate(n);
            if tamper {
                cipher[0] ^= 1;
            }
            write_frame(&mut source, &cipher).await.unwrap();
            if tamper {
                assert!(read_message(&mut destination, &sb).await.is_err());
            } else {
                assert_eq!(
                    read_message(&mut destination, &sb).await.unwrap(),
                    Message::Pong(42)
                );
                write_frame(&mut source, &cipher).await.unwrap();
                assert!(read_message(&mut destination, &sb).await.is_err());
            }
        }
    }
}
