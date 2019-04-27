use crate::ts::TSPacket;
use std::collections::hash_map::{Entry, HashMap};
use std::error::Error as StdError;
use std::fmt::{Display, Error, Formatter};
use std::sync::Arc;
use std::sync::Mutex;
use tokio::prelude::{Async, AsyncSink, Sink};
use tokio_channel::mpsc::{channel, Receiver, Sender};

struct Inner {
    senders: HashMap<u16, Sender<TSPacket>>,
    closed: bool,
}

impl Inner {
    fn new() -> Inner {
        Inner {
            senders: HashMap::new(),
            closed: false,
        }
    }
}

pub struct Register {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Debug)]
pub enum RegistrationError {
    AlreadyRegistered,
    Closed,
}

impl RegistrationError {
    pub fn is_closed(&self) -> bool {
        match self {
            RegistrationError::Closed => true,
            _ => false,
        }
    }
}

impl Display for RegistrationError {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        write!(f, "{:?}", self)
    }
}

impl StdError for RegistrationError {}

impl Register {
    pub fn try_register(&mut self, pid: u16) -> Result<Receiver<TSPacket>, RegistrationError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closed {
            return Err(RegistrationError::Closed);
        }
        match inner.senders.entry(pid) {
            Entry::Vacant(entry) => {
                let (tx, rx) = channel(1);
                entry.insert(tx);
                Ok(rx)
            }
            _ => Err(RegistrationError::AlreadyRegistered),
        }
    }
}

impl Clone for Register {
    fn clone(&self) -> Register {
        Register {
            inner: self.inner.clone(),
        }
    }
}

pub struct Demuxer {
    inner: Arc<Mutex<Inner>>,
}

impl Demuxer {
    pub fn new() -> Demuxer {
        Demuxer {
            inner: Arc::new(Mutex::new(Inner::new())),
        }
    }

    pub fn register(&self) -> Register {
        Register {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Debug)]
pub struct DemuxError(TSPacket);

impl DemuxError {
    fn into_packet(self) -> TSPacket {
        self.0
    }
}

impl Display for DemuxError {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        write!(f, "failed to demux")
    }
}

impl StdError for DemuxError {}

impl Drop for Demuxer {
    fn drop(&mut self) {
        let mut inner = self.inner.lock().unwrap();
        inner.senders.clear();
        inner.closed = true;
    }
}

impl Sink for Demuxer {
    type SinkItem = TSPacket;
    type SinkError = DemuxError;

    fn start_send(
        &mut self,
        item: Self::SinkItem,
    ) -> Result<AsyncSink<Self::SinkItem>, Self::SinkError> {
        let mut inner = self.inner.lock().unwrap();
        let pid = item.pid;
        match inner.senders.get_mut(&pid) {
            Some(sender) => match sender.start_send(item) {
                Ok(p) => Ok(p),
                Err(_) => {
                    // when sender returns an error, the channel is closed.
                    Ok(AsyncSink::Ready)
                }
            },
            None => Ok(AsyncSink::Ready),
        }
    }

    fn poll_complete(&mut self) -> Result<Async<()>, Self::SinkError> {
        Ok(Async::Ready(()))
    }
}
