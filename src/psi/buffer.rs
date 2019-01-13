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
    Skipping(u8),
    FindingLength,
    Buffering(u16),
    Proceeded,
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
            let packet = match self.inner.poll() {
                Ok(Async::Ready(Some(packet))) => packet,
                Ok(Async::Ready(None)) => return Ok(Async::Ready(None)),
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(e) => bail!("some error {:?}", e),
            };

            if packet.transport_error_indicator {
                continue;
            }
            if packet.data.is_none() {
                bail!("malformed psi packet, no data")
            }
            let mut bytes = &packet.data.unwrap()[..];

            if packet.payload_unit_start_indicator {
                let pointer_field = bytes[0];
                self.state = State::Skipping(pointer_field + 1);
                self.counter = packet.continuity_counter;
                self.buf.clear();
            } else {
                match self.state {
                    State::Initial => {
                        // seen partial section
                        continue;
                    }
                    State::Proceeded => {
                        // already completed
                        continue;
                    }
                    _ => {
                        if self.counter == packet.continuity_counter {
                            // duplicate packet
                            continue;
                        } else if (self.counter + 1) % 16 == packet.continuity_counter {
                            self.counter = packet.continuity_counter;
                        } else {
                            self.state = State::Initial;
                            bail!("psi packet discontinued");
                        }
                    }
                }
            }

            if let State::Skipping(n) = self.state {
                if bytes.len() < (n as usize) {
                    self.state = State::Skipping(n - (bytes.len() as u8));
                    continue;
                } else {
                    self.state = State::FindingLength;
                    bytes = &bytes[n as usize..];
                }
            }

            self.buf.extend_from_slice(bytes);

            if let State::FindingLength = self.state {
                if self.buf.len() < 3 {
                    continue;
                } else {
                    let section_length =
                        (u16::from(self.buf[1] & 0xf) << 8) | u16::from(self.buf[2]);
                    self.state = State::Buffering(section_length + 3);
                }
            }
            if let State::Buffering(length) = self.state {
                if self.buf.len() >= (length as usize) {
                    self.state = State::Proceeded;
                    return Ok(Async::Ready(Some(
                        self.buf.split_to(length as usize).freeze(),
                    )));
                }
            }
        }
    }
}
