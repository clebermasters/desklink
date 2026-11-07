use crate::config::Config;
use anyhow::{Context, Result};
use desklink_clipboard::ClipboardService;
use desklink_core::{ClipboardClock, Effect, Engine};
use desklink_platform::{Desktop, LocalEvent};
use desklink_protocol::{Message, VERSION};
use desklink_transport::{Identity, NetworkEvent, PeerHandle, PeerKey, parse_public};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Semaphore, mpsc},
    task::JoinSet,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

struct Connected {
    handle: PeerHandle,
    clipboard: bool,
}

pub async fn run(config: Config, identity: Identity) -> Result<()> {
    let edges: Vec<_> = config.peers.iter().filter_map(|p| p.edge).collect();
    let mut desktop = desklink_platform::open(&edges, config.clipboard)?;
    let geometry = desktop.geometry()?;
    let mut clipboard = if config.clipboard && !desktop.owns_clipboard() {
        Some(ClipboardService::start().context("clipboard unavailable; configure clipboard=false or use --no-clipboard to run without it")?)
    } else {
        None
    };
    let trusted: HashSet<_> = config
        .peers
        .iter()
        .map(|p| parse_public(&p.public_key))
        .collect::<Result<_>>()?;
    anyhow::ensure!(
        !trusted.contains(&identity.public),
        "configuration pairs this computer with itself"
    );
    let routes = config
        .peers
        .iter()
        .filter_map(|p| p.edge.map(|edge| (edge, &p.public_key)))
        .map(|(edge, key)| Ok((edge, parse_public(key)?)))
        .collect::<Result<Vec<_>>>()?;
    let identity = Arc::new(identity);
    let trusted = Arc::new(trusted);
    let mut engine = Engine::new(identity.public, geometry, routes);
    let mut clock = ClipboardClock::new(identity.public);
    let mut peers: HashMap<PeerKey, Connected> = HashMap::new();
    let active = Arc::new(Mutex::new(HashSet::new()));
    let (tx, mut events) = mpsc::channel(512);
    let shutdown = CancellationToken::new();
    let hello = Message::Hello {
        version: VERSION,
        name: config.name.clone(),
        geometry,
        clipboard: config.clipboard,
    };
    let listener = TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.listen))?;
    let mut tasks = JoinSet::new();
    let listen_cancel = shutdown.clone();
    let listen_id = identity.clone();
    let listen_trusted = trusted.clone();
    let listen_hello = hello.clone();
    let listen_tx = tx.clone();
    tasks.spawn(async move {
        let slots=Arc::new(Semaphore::new(32));let mut sessions=JoinSet::new();
        loop {
            tokio::select! {
                _=listen_cancel.cancelled()=>break,
                _=sessions.join_next(),if !sessions.is_empty()=>{

                },
                accepted=listener.accept()=>match accepted {
                    Ok((socket,_))=>{
                    let Ok(permit) = slots.clone().try_acquire_owned() else {
                        continue;
                    };
                    let (id, trust, hello, tx, cancel) = (
                        listen_id.clone(),
                        listen_trusted.clone(),
                        listen_hello.clone(),
                        listen_tx.clone(),
                        listen_cancel.clone(),
                    );
                    sessions.spawn(async move {
                        let _permit = permit;
                        if let Err(error) =
                            desklink_transport::session(socket, false, id, trust, hello, tx, cancel).await
                        {
                            tracing::debug!(%error,"incoming session ended");
                        }
                    });
                }
                    Err(error)=>{
                    tracing::error!(%error,"listener failed");
                    listen_cancel.cancel();
                    break;
                }
                }
            }
        }
        sessions.abort_all();
    });
    for peer in &config.peers {
        if let Some(address) = &peer.address {
            let address = address.clone();
            let key = parse_public(&peer.public_key)?;
            let (id, trust, hello, tx, cancel, active) = (
                identity.clone(),
                trusted.clone(),
                hello.clone(),
                tx.clone(),
                shutdown.clone(),
                active.clone(),
            );
            tasks.spawn(async move {
                loop {
                    if cancel.is_cancelled(){break;}
                    let connected=active.lock().is_ok_and(|p|p.contains(&key));
                    if !connected {
                        match timeout(Duration::from_secs(4),TcpStream::connect(&address)).await {
                            Ok(Ok(socket))=>if let Err(error)=desklink_transport::session(socket,true,id.clone(),trust.clone(),hello.clone(),tx.clone(),cancel.clone()).await {tracing::debug!(%error,%address,"outgoing session ended");},
                            Ok(Err(error))=>tracing::debug!(%error,%address,"peer unavailable"),
                            Err(_)=>tracing::debug!(%address,"connection timed out"),
                        }
                    }
                    tokio::select! {_=cancel.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(2))=>{

                        }}
                }
            });
        }
    }
    tracing::info!(name=%config.name,listen=%config.listen,clipboard=config.clipboard,"sharing started; Ctrl+Alt+Shift+Escape restores local control");
    let mut tick = tokio::time::interval(Duration::from_millis(4));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut geometry_tick = tokio::time::interval(Duration::from_secs(2));
    let mut last_geometry = geometry;
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result:Result<()>=async {
        loop {
            let effects=tokio::select! {
                _=tokio::signal::ctrl_c()=>break,
                _=term.recv()=>break,
                _=shutdown.cancelled()=>break,
                _=tick.tick()=>{
                    let mut effects = engine.tick();
                    for event in desktop.poll()? {
                        match event {
                            LocalEvent::Pointer { x, y, dx, dy } => effects.extend(engine.pointer(x, y, dx, dy)),
                            LocalEvent::Input(input) => effects.extend(match engine.local_input(input) {
                                Ok(e) => e,
                                Err(error) => {
                                    tracing::warn!(%error,"input sharing reset");
                                    engine.abort()
                                }
                            }),
                            LocalEvent::Clipboard(content) => {
                                if config.clipboard
                                    && let Some(transfer) = clock.local(content)
                                {
                                    for peer in peers.values().filter(|p| p.clipboard) {
                                        peer.handle.clipboard(transfer.clone());
                                    }
                                }
                            }
                            LocalEvent::CaptureLost => effects.extend(engine.abort()),
                            LocalEvent::Failure(message) => anyhow::bail!(message),
                        }
                    }
                    effects
                }
                _=geometry_tick.tick()=>{
                    let g = desktop.geometry()?;
                    if g != last_geometry {
                        last_geometry = g;
                        engine.geometry_changed(g)
                    } else {
                        Vec::new()
                    }
                }
                content=clipboard_update(&mut clipboard)=>{
                    if let Some(transfer) = clock.local(content?)
                    {
                        tracing::debug!(kind=?transfer.content.kind,bytes=transfer.content.bytes.len(),"local clipboard changed");
                        for peer in peers.values().filter(|p| p.clipboard) {
                            peer.handle.clipboard(transfer.clone());
                        }
                    }
                    Vec::new()
                }
                event=events.recv()=>{
                    let Some(event) = event else {
                        break;
                    };
                    match event {
                        NetworkEvent::Connected { key, hello, handle } => {
                            let preferred = handle.outbound == (identity.public < key);
                            if let Some(existing) = peers.get(&key)
                                && (!preferred || existing.handle.outbound == (identity.public < key))
                            {
                                handle.cancel.cancel();
                                continue;
                            }
                            let effects = if let Some(old) = peers.remove(&key) {
                                old.handle.cancel.cancel();
                                engine.disconnected(key)
                            } else {
                                Vec::new()
                            };
                            let (name, can_clipboard) = match hello {
                                Message::Hello {
                                    name, clipboard, ..
                                } => (name, clipboard),
                                _ => unreachable!(),
                            };
                            if can_clipboard && let Some(content) = clock.current() {
                                handle.clipboard(content);
                            }
                            tracing::info!(peer=%name,key=%hex::encode(key),"peer connected");
                            peers.insert(
                                key,
                                Connected {
                                    handle,
                                    clipboard: can_clipboard,
                                },
                            );
                            active
                                .lock()
                                .map_err(|_| anyhow::anyhow!("connection state poisoned"))?
                                .insert(key);
                            engine.connected(key);
                            effects
                        }
                        NetworkEvent::Disconnected { key, id } => {
                            if peers.get(&key).is_none_or(|p| p.handle.id != id) {
                                continue;
                            }
                            peers.remove(&key);
                            active
                                .lock()
                                .map_err(|_| anyhow::anyhow!("connection state poisoned"))?
                                .remove(&key);
                            tracing::info!(key=%hex::encode(key),"peer disconnected; input released");
                            engine.disconnected(key)
                        }
                        NetworkEvent::Message { key, id, message } => {
                            if peers.get(&key).is_none_or(|p| p.handle.id != id) {
                                continue;
                            }
                            match engine.remote(key, message) {
                                Ok(e) => e,
                                Err(error) => {
                                    tracing::warn!(%error,"invalid peer control message");
                                    peers[&key].handle.cancel.cancel();
                                    engine.disconnected(key)
                                }
                            }
                        }
                        NetworkEvent::Clipboard { key, id, transfer } => {
                            if !config.clipboard || peers.get(&key).is_none_or(|p| p.handle.id != id) {
                                continue;
                            }
                            if let Some(transfer) = clock.remote(transfer) {
                                if desktop.owns_clipboard() {
                                    desktop.clipboard_apply(transfer.content.clone())?;
                                } else if let Some(clipboard) = &clipboard {
                                    clipboard.apply(transfer.content.clone());
                                }
                                for (other, peer) in peers.iter().filter(|(_, p)| p.clipboard) {
                                    if *other != key {
                                        peer.handle.clipboard(transfer.clone());
                                    }
                                }
                                tracing::info!(kind=?transfer.content.kind,bytes=transfer.content.bytes.len(),"remote clipboard applied");
                            }
                            Vec::new()
                        }
                    }
                }
            };
            if let Err(error)=apply(effects,&mut engine,&mut *desktop,&peers) {
                tracing::warn!(%error,"desktop action failed; restoring local control");
                let recovery=engine.abort();let _=apply(recovery,&mut engine,&mut *desktop,&peers);
            }
        }Ok(())
    }.await;
    let effects = engine.abort();
    let _ = apply(effects, &mut engine, &mut *desktop, &peers);
    let _ = desktop.release();
    shutdown.cancel();
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    tracing::info!("sharing stopped");
    result
}

async fn clipboard_update(
    clipboard: &mut Option<ClipboardService>,
) -> Result<desklink_protocol::ClipboardContent> {
    match clipboard {
        Some(c) => c
            .next_change()
            .await
            .context("native clipboard worker stopped"),
        None => std::future::pending().await,
    }
}

fn apply(
    effects: Vec<Effect>,
    engine: &mut Engine,
    desktop: &mut dyn Desktop,
    peers: &HashMap<PeerKey, Connected>,
) -> Result<()> {
    let recovery = effects.iter().any(|e| matches!(e, Effect::ReleaseCapture));
    let mut first_error = None;
    let mut pending: VecDeque<_> = effects.into();
    while let Some(effect) = pending.pop_front() {
        let result = (|| -> Result<()> {
            match effect {
                Effect::Send(key, message) => {
                    if let Some(peer) = peers.get(&key) {
                        peer.handle.send(message)?;
                    }
                }
                Effect::Capture => {
                    for input in desktop.capture()? {
                        pending.extend(engine.local_input(input)?);
                    }
                }
                Effect::ReleaseCapture => desktop.release()?,
                Effect::Warp(x, y) => desktop.warp(x, y)?,
                Effect::Inject(input) => desktop.inject(input)?,
                Effect::Status(status) => tracing::info!(status),
            }
            Ok(())
        })();
        if let Err(error) = result {
            if !recovery {
                return Err(error);
            }
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use desklink_protocol::{Geometry, Input};
    struct FaultyDesktop(Vec<Input>);
    impl Desktop for FaultyDesktop {
        fn geometry(&mut self) -> Result<Geometry> {
            unreachable!()
        }
        fn poll(&mut self) -> Result<Vec<LocalEvent>> {
            unreachable!()
        }
        fn capture(&mut self) -> Result<Vec<Input>> {
            unreachable!()
        }
        fn release(&mut self) -> Result<()> {
            Ok(())
        }
        fn warp(&mut self, _: i32, _: i32) -> Result<()> {
            Ok(())
        }
        fn inject(&mut self, input: Input) -> Result<()> {
            if matches!(input, Input::Key { code: 464, .. }) {
                anyhow::bail!("unsupported key");
            }
            self.0.push(input);
            Ok(())
        }
    }
    #[test]
    fn cleanup_continues_after_one_release_fails() {
        let mut desktop = FaultyDesktop(Vec::new());
        let mut engine = Engine::new(
            [0; 32],
            Geometry {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            vec![],
        );
        let release = Input::Button {
            button: 1,
            down: false,
        };
        assert!(
            apply(
                vec![
                    Effect::ReleaseCapture,
                    Effect::Inject(Input::Key {
                        code: 464,
                        down: false
                    }),
                    Effect::Inject(release)
                ],
                &mut engine,
                &mut desktop,
                &HashMap::new()
            )
            .is_err()
        );
        assert_eq!(desktop.0, vec![release]);
    }
}
