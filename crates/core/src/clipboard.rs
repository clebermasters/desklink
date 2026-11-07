use desklink_protocol::{ClipboardContent, ClipboardTransfer, Stamp};
use std::sync::Arc;

pub struct ClipboardClock {
    own: [u8; 32],
    counter: u64,
    current: Option<Arc<ClipboardTransfer>>,
}
impl ClipboardClock {
    pub fn new(own: [u8; 32]) -> Self {
        Self {
            own,
            counter: 0,
            current: None,
        }
    }
    pub fn current(&self) -> Option<Arc<ClipboardTransfer>> {
        self.current.clone()
    }
    pub fn local(&mut self, content: ClipboardContent) -> Option<Arc<ClipboardTransfer>> {
        if !content.valid()
            || self
                .current
                .as_ref()
                .is_some_and(|c| c.content.hash() == content.hash())
        {
            return None;
        }
        self.counter = self.counter.checked_add(1)?;
        let t = Arc::new(ClipboardTransfer {
            stamp: Stamp {
                counter: self.counter,
                origin: self.own,
            },
            content,
        });
        self.current = Some(t.clone());
        Some(t)
    }
    pub fn remote(&mut self, transfer: ClipboardTransfer) -> Option<Arc<ClipboardTransfer>> {
        if !transfer.content.valid()
            || transfer.stamp.counter == u64::MAX
            || self
                .current
                .as_ref()
                .is_some_and(|c| c.stamp >= transfer.stamp)
        {
            return None;
        }
        self.counter = self.counter.max(transfer.stamp.counter);
        let t = Arc::new(transfer);
        self.current = Some(t.clone());
        Some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desklink_protocol::ClipboardKind;
    fn text(v: &str) -> ClipboardContent {
        ClipboardContent {
            kind: ClipboardKind::Text,
            bytes: v.as_bytes().to_vec(),
        }
    }
    #[test]
    fn simultaneous_updates_converge_and_remote_apply_does_not_echo() {
        let mut a = ClipboardClock::new([1; 32]);
        let mut b = ClipboardClock::new([2; 32]);
        let aa = a.local(text("A")).unwrap();
        let bb = b.local(text("B")).unwrap();
        a.remote((*bb).clone());
        assert!(b.remote((*aa).clone()).is_none());
        assert_eq!(a.current().unwrap().content, b.current().unwrap().content);
        assert!(a.local(text("B")).is_none());
        let newer = a.local(text("C")).unwrap();
        assert!(b.remote((*newer).clone()).is_some());
    }
}
