use bytes::{Bytes, BytesMut};
use failure;
use failure::bail;
use std::fmt::Debug;
use tokio::prelude::{Async, Stream};

use crate::ts;

const INITIAL_BUFFER: usize = 4096;

#[derive(Debug)]
enum State {
    Initial,
    Buffering,
    Closed,
}

#[derive(Debug)]
pub struct Buffer<S> {
    inner: S,
    state: State,
    counter: u8,
    buf: BytesMut,
}

impl<S> Buffer<S> {
    pub fn new(stream: S) -> Self {
        Buffer {
            inner: stream,
            state: State::Initial,
            counter: 0,
            buf: BytesMut::with_capacity(INITIAL_BUFFER),
        }
    }

    pub fn into_inner(self) -> S {
        self.inner
    }

    fn get_bytes(&mut self) -> Result<Bytes, failure::Error> {
        if self.buf.len() < 6 {
            bail!("not enough data");
        }
        let pes_packet_length = (usize::from(self.buf[4]) << 8) | usize::from(self.buf[5]);
        if pes_packet_length == 0 {
            return Ok(self.buf.take().freeze());
        }
        if self.buf.len() < pes_packet_length + 6 {
            bail!("not enough data");
        }
        return Ok(self.buf.split_to(pes_packet_length + 6).freeze());
    }
}

impl<S, E> Stream for Buffer<S>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    type Item = Bytes;
    type Error = failure::Error;

    fn poll(&mut self) -> Result<Async<Option<Self::Item>>, Self::Error> {
        loop {
            if let State::Closed = self.state {
                return Ok(Async::Ready(None));
            }

            let packet = match self.inner.poll() {
                Ok(Async::Ready(Some(packet))) => packet,
                Ok(Async::Ready(None)) => {
                    self.state = State::Closed;
                    if let State::Buffering = self.state {
                        return Ok(Async::Ready(Some(self.get_bytes()?)));
                    }
                    return Ok(Async::Ready(None));
                }
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(e) => bail!("some error {:?}", e),
            };

            if packet.transport_error_indicator {
                continue;
            }

            let data = match packet.data {
                Some(ref data) => data.as_ref(),
                None => bail!("no data"),
            };

            if packet.payload_unit_start_indicator {
                let mut bytes = None;
                if let State::Buffering = self.state {
                    bytes = Some(self.get_bytes());
                }

                self.state = State::Buffering;
                self.counter = packet.continuity_counter;
                self.buf.clear();
                self.buf.extend_from_slice(data);

                return match bytes {
                    Some(Ok(bytes)) => Ok(Async::Ready(Some(bytes))),
                    Some(Err(e)) => Err(e),
                    None => continue,
                };
            } else {
                if let State::Initial = self.state {
                    // seen partial packet
                    continue;
                }

                if self.counter == packet.continuity_counter {
                    // duplicate packet
                    continue;
                } else if (self.counter + 1) % 16 == packet.continuity_counter {
                    self.counter = packet.continuity_counter;
                } else {
                    self.state = State::Initial;
                    self.buf.clear();
                    bail!("pes packet discontinued");
                }

                self.buf.extend_from_slice(data);
            }
        }
    }
}
