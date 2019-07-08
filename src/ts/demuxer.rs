use crate::ts::TSPacket;
use std::collections::hash_map::{Entry as HashEntry, HashMap};
use tokio::prelude::{Async, AsyncSink, Future, IntoFuture, Sink};

pub struct Demuxer<Fun, F, S> {
    sinks: HashMap<u16, Option<S>>,
    sink_making_future: Option<(u16, F)>,
    sink_maker: Fun,
}

impl<Fun, F, S> Demuxer<Fun, F, S> {
    pub fn new(f: Fun) -> Self {
        Self {
            sinks: HashMap::new(),
            sink_making_future: None,
            sink_maker: f,
        }
    }
}

impl<Func, IF, S> Sink for Demuxer<Func, IF::Future, S>
where
    Func: FnMut(u16) -> IF,
    IF: IntoFuture<Item = Option<S>>,
    S: Sink<SinkItem = TSPacket, SinkError = IF::Error>,
{
    type SinkItem = TSPacket;
    type SinkError = IF::Error;

    fn start_send(
        &mut self,
        item: Self::SinkItem,
    ) -> Result<AsyncSink<Self::SinkItem>, Self::SinkError> {
        if let Some((pid, mut future)) = self.sink_making_future.take() {
            match future.poll() {
                Ok(Async::Ready(Some(sink))) => {
                    self.sinks.insert(pid, Some(sink));
                }
                Ok(Async::Ready(None)) => {
                    return Ok(AsyncSink::Ready);
                }
                Ok(Async::NotReady) => {
                    self.sink_making_future = Some((pid, future));
                    return Ok(AsyncSink::NotReady(item));
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        let sink = match self.sinks.entry(item.pid) {
            HashEntry::Occupied(e) => e.into_mut(),
            HashEntry::Vacant(e) => {
                let mut future = (self.sink_maker)(item.pid).into_future();
                match future.poll() {
                    Ok(Async::Ready(Some(sink))) => e.insert(Some(sink)),
                    Ok(Async::Ready(None)) => {
                        return Ok(AsyncSink::Ready);
                    }
                    Ok(Async::NotReady) => {
                        self.sink_making_future = Some((item.pid, future));
                        return Ok(AsyncSink::NotReady(item));
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
        };

        match sink {
            Some(s) => match s.start_send(item) {
                Ok(p) => Ok(p),
                Err(_) => {
                    // When a sink returns an error, the sink becomes permanently unavailable.
                    // BTW, this demuxer is working, the error is ignored.
                    *sink = None;
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
