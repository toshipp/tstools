use std::fmt::Debug;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use thiserror;
use tokio_stream::Stream;

use crate::ts;

const INITIAL_BUFFER: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum BufferError {
    #[error("malformed psi packet, no data")]
    MalformedNoData,
    #[error("malformed psi packet, no section header in the packet")]
    MalformedNoSectionHeader,
    #[error("discontinued psi packet")]
    Discontinued,
}

#[derive(Debug)]
enum State {
    Initial,
    Partial,
    Full,
}

pub struct Buffer<S> {
    s: S,
    state: State,
    counter: u8,
    buf: BytesMut,
}

impl<S> Buffer<S> {
    pub fn new(stream: S) -> Self {
        Buffer {
            s: stream,
            state: State::Initial,
            counter: 0,
            buf: BytesMut::with_capacity(INITIAL_BUFFER),
        }
    }

    fn feed_packet(&mut self, packet: ts::TSPacket) -> Result<(), BufferError> {
        let bytes = match packet.data {
            Some(ref data) => data.as_ref(),
            None => return Err(BufferError::MalformedNoData),
        };
        if packet.payload_unit_start_indicator {
            let pointer_field = usize::from(bytes[0]);
            if bytes.len() < pointer_field + 1 {
                return Err(BufferError::MalformedNoSectionHeader);
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
                return Err(BufferError::Discontinued);
            }
            self.buf.extend_from_slice(bytes);
        }
        Ok(())
    }
}

impl<S> Stream for Buffer<S>
where
    S: Stream<Item = ts::TSPacket> + Unpin,
{
    type Item = Result<Bytes, BufferError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        macro_rules! next_valid_packet {
            () => {{
                loop {
                    let packet = match Pin::new(&mut self.s).poll_next(cx) {
                        Poll::Ready(Some(packet)) => packet,
                        Poll::Ready(None) => return Poll::Ready(None),
                        Poll::Pending => return Poll::Pending,
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
                    return Poll::Ready(Some(Ok(buf)));
                }
            }
        }
    }
}
