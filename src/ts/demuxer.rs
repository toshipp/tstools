use crate::ts::TSPacket;
use std::collections::hash_map::{Entry as HashEntry, HashMap};
use std::error::Error as StdError;
use std::fmt::{Display, Error, Formatter};
use std::sync::Arc;
use std::sync::Mutex;
use tokio::prelude::{Async, AsyncSink, Sink};
use tokio::sync::mpsc::{channel, Receiver, Sender};

struct Entry {
    sender: Sender<TSPacket>,
    closed: bool,
}

struct Inner {
    num: usize,
    senders: HashMap<u16, Entry>,
    closed: bool,
}

impl Inner {
    fn new() -> Inner {
        Inner {
            num: 0,
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
        let ret = match inner.senders.entry(pid) {
            HashEntry::Vacant(entry) => {
                let (tx, rx) = channel(1);
                entry.insert(Entry {
                    sender: tx,
                    closed: false,
                });
                Ok(rx)
            }
            _ => Err(RegistrationError::AlreadyRegistered),
        };
        if ret.is_ok() {
            inner.num += 1;
        }
        ret
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
pub enum DemuxError {
    Closed,
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
        if inner.num == 0 {
            return Err(DemuxError::Closed);
        }
        let pid = item.pid;
        let mut is_err = false;
        let ret = match inner.senders.get_mut(&pid) {
            Some(entry) => {
                if entry.closed {
                    Ok(AsyncSink::Ready)
                } else {
                    match entry.sender.start_send(item) {
                        Ok(p) => Ok(p),
                        Err(_) => {
                            // when sender returns an error, the channel is closed.
                            entry.closed = true;
                            is_err = true;
                            Ok(AsyncSink::Ready)
                        }
                    }
                }
            }
            None => Ok(AsyncSink::Ready),
        };
        if is_err {
            inner.num -= 1;
        }
        return ret;
    }

    fn poll_complete(&mut self) -> Result<Async<()>, Self::SinkError> {
        Ok(Async::Ready(()))
    }
}
