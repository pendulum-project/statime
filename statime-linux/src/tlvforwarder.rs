use statime::port::{ForwardedTLV, ForwardedTLVProvider};

pub struct TlvForwarder {
    sender: tokio::sync::broadcast::Sender<ForwardedTLV<'static>>,
    receiver: tokio::sync::broadcast::Receiver<ForwardedTLV<'static>>,
    peek: Option<ForwardedTLV<'static>>,
}

impl Default for TlvForwarder {
    fn default() -> Self {
        Self::new()
    }
}

impl TlvForwarder {
    pub fn new() -> Self {
        let (sender, receiver) = tokio::sync::broadcast::channel(128);
        Self {
            sender,
            receiver,
            peek: None,
        }
    }

    pub fn forward(&self, tlv: ForwardedTLV<'static>) {
        // Dont care about all receivers being gone.
        let _ = self.sender.send(tlv);
    }

    // We have this instead of clone since this a duplication of this
    // basically creates a second forwarder that starts reading at the
    // same point, without updating the old. This is in many cases not
    // what the user wants so extra friction here is good.
    pub fn duplicate(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            receiver: self.receiver.resubscribe(),
            peek: None,
        }
    }

    pub fn empty(&mut self) {
        use tokio::sync::broadcast::error::TryRecvError;
        self.peek = None;
        // Empty the receiver
        while !matches!(
            self.receiver.try_recv(),
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed)
        ) {}
    }
}

impl ForwardedTLVProvider for TlvForwarder {
    fn next_if_smaller(&mut self, max_size: usize) -> Option<statime::port::ForwardedTLV> {
        use tokio::sync::broadcast::error::TryRecvError;

        while self.peek.is_none() {
            match self.receiver.try_recv() {
                Ok(value) => self.peek = Some(value),
                Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => return None,
                Err(TryRecvError::Lagged(_)) => continue,
            }
        }

        // workaround for take_if not being stable
        if let Some(v) = self.peek.take() {
            if v.size() <= max_size {
                Some(v)
            } else {
                self.peek = Some(v);
                None
            }
        } else {
            None
        }
    }
}
