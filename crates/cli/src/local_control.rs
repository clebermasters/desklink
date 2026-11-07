use anyhow::{Result, ensure};
use desklink_settings::control::{MAX_CONTROL, Request, Status, socket_path};
use std::sync::Arc;
use std::{
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    sync::{Semaphore, mpsc, oneshot},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

pub struct Envelope {
    pub request: Request,
    pub reply: oneshot::Sender<Status>,
    pub sent: oneshot::Receiver<()>,
}
pub struct Server {
    pub incoming: mpsc::Receiver<Envelope>,
    path: PathBuf,
    cancel: CancellationToken,
}

impl Server {
    pub fn start(config: &Path) -> Result<Self> {
        let path = socket_path(config);
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            ensure!(
                metadata.file_type().is_socket(),
                "Control path is occupied by a non-socket file."
            );
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let (tx, incoming) = mpsc::channel(16);
        let cancel = CancellationToken::new();
        let stopped = cancel.clone();
        tokio::spawn(async move {
            let slots = Arc::new(Semaphore::new(16));
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    _=stopped.cancelled()=>break,
                    _=clients.join_next(),if !clients.is_empty()=>{},
                    accepted=listener.accept()=>{
                        let Ok((mut socket,_))=accepted else{break;};
                        let Ok(permit)=slots.clone().try_acquire_owned()else{continue;};let tx=tx.clone();
                        clients.spawn(async move {
                            let _permit=permit;
                            let result:Result<()>=tokio::time::timeout(Duration::from_secs(2),async {
                                let n=socket.read_u32().await? as usize;ensure!(n>0&&n<=MAX_CONTROL,"Invalid control frame.");
                                let mut bytes=vec![0;n];socket.read_exact(&mut bytes).await?;let request=serde_json::from_slice(&bytes)?;
                                let(reply,response)=oneshot::channel();let(sent_tx,sent)=oneshot::channel();
                                tx.send(Envelope {request,reply,sent}).await?;
                                let response=response.await?;let bytes=serde_json::to_vec(&response)?;
                                socket.write_u32(bytes.len() as u32).await?;socket.write_all(&bytes).await?;let _=sent_tx.send(());Ok(())
                            }).await.unwrap_or_else(|e|Err(e.into()));
                            if let Err(error)=result {tracing::debug!(%error,"local control request ended");}
                        });
                    }
                }
            }
            clients.abort_all();
        });
        Ok(Self {
            incoming,
            path,
            cancel,
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
        let _ = std::fs::remove_file(&self.path);
    }
}
