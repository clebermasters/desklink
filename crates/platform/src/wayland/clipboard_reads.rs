//! A completed but unconsumed read can outlive clipboard ownership; cancellation alone is insufficient.
use anyhow::Result;
use desklink_protocol::ClipboardContent;
use std::future::Future;
use tokio::task::{AbortHandle, JoinSet};

#[derive(Default)]
pub struct ClipboardReads {
    revision: u64,
    tasks: JoinSet<(u64, Result<ClipboardContent>)>,
}

impl ClipboardReads {
    pub fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.tasks.abort_all();
    }
    pub fn start(
        &mut self,
        read: impl Future<Output = Result<ClipboardContent>> + Send + 'static,
    ) -> AbortHandle {
        let revision = self.revision;
        self.tasks.spawn(async move { (revision, read.await) })
    }
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
    pub async fn next(&mut self) -> Option<Result<ClipboardContent>> {
        while let Some(result) = self.tasks.join_next().await {
            match result {
                Ok((revision, content)) if revision == self.revision => return Some(content),
                Err(error) if !error.is_cancelled() => return Some(Err(error.into())),
                _ => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desklink_protocol::ClipboardKind;
    fn text(value: &str) -> ClipboardContent {
        ClipboardContent {
            kind: ClipboardKind::Text,
            bytes: value.as_bytes().to_vec(),
        }
    }

    #[tokio::test]
    async fn completed_reads_from_previous_owner_cannot_overwrite_a_remote_copy() {
        let mut reads = ClipboardReads::default();
        let handle = reads.start(async { Ok(text("completed stale read")) });
        while !handle.is_finished() {
            tokio::task::yield_now().await;
        }
        reads.invalidate();
        assert!(reads.next().await.is_none());
        reads.start(async { Ok(text("new owner")) });
        assert_eq!(reads.next().await.unwrap().unwrap(), text("new owner"));
    }
}
