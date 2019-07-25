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
    Partial,
    Full,
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
        return self.inner;
    }
}

impl<S, E> Buffer<S>
where
    S: Stream<Item = ts::TSPacket, Error = E>,
    E: Debug,
{
    fn feed_packet(&mut self, packet: ts::TSPacket) -> Result<(), failure::Error> {
        let bytes = match packet.data {
            Some(ref data) => data.as_ref(),
            None => bail!("malformed psi packet, no data"),
        };
        if packet.payload_unit_start_indicator {
            let pointer_field = usize::from(bytes[0]);
            if bytes.len() < pointer_field + 1 {
                bail!("malformed psi packet, no section header in the packet");
            }
            self.buf.clear();
            self.buf.extend_from_slice(&bytes[pointer_field + 1..]);
            self.counter = packet.continuity_counter;
            self.state = State::Partial;
        } else {
            if self.counter == packet.continuity_counter {
                // duplicate packet, do nothing.
                return Ok(());
            } else if (self.counter + 1) % 16 == packet.continuity_counter {
                self.counter = packet.continuity_counter;
            } else {
                self.state = State::Initial;
                bail!("psi packet discontinued");
            }
            self.buf.extend_from_slice(bytes);
        }
        Ok(())
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
        macro_rules! next_valid_packet {
            () => {{
                loop {
                    let packet = match self.inner.poll() {
                        Ok(Async::Ready(Some(packet))) => packet,
                        Ok(Async::Ready(None)) => return Ok(Async::Ready(None)),
                        Ok(Async::NotReady) => return Ok(Async::NotReady),
                        Err(e) => bail!("some error {:?}", e),
                    };
                    if !packet.transport_error_indicator {
                        break packet;
                    }
                }
            }};
        }

        loop {
            match self.state {
                State::Initial => {
                    let packet = next_valid_packet!();
                    if packet.payload_unit_start_indicator {
                        self.feed_packet(packet)?;
                    }
                }
                State::Partial => {
                    if self.buf.len() < 3 {
                        // not sufficient data for psi header.
                        let packet = next_valid_packet!();
                        self.feed_packet(packet)?;
                        continue;
                    }
                    let section_length =
                        (usize::from(self.buf[1] & 0xf) << 8) | usize::from(self.buf[2]);
                    if self.buf.len() < section_length + 3 {
                        let packet = next_valid_packet!();
                        self.feed_packet(packet)?;
                        continue;
                    }
                    self.state = State::Full;
                }
                State::Full => {
                    self.state = State::Partial;
                    let section_length =
                        (usize::from(self.buf[1] & 0xf) << 8) | usize::from(self.buf[2]);
                    let buf = self.buf.split_to(section_length + 3).freeze();
                    return Ok(Async::Ready(Some(buf)));
                }
            }
        }
    }
}
