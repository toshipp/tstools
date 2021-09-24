use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio_stream::Stream;

pub struct Cueable<S>
where
    S: Stream,
{
    s: S,
    items: VecDeque<S::Item>,
}

pub struct Cued<S>
where
    S: Stream,
{
    s: S,
    items: VecDeque<S::Item>,
}

pub fn cueable<S: Stream>(s: S) -> Cueable<S> {
    Cueable {
        s,
        items: VecDeque::new(),
    }
}

impl<S> Cueable<S>
where
    S: Stream,
{
    pub fn cue_up(self) -> Cued<S> {
        Cued {
            s: self.s,
            items: self.items,
        }
    }
}

impl<S, I> Stream for Cueable<S>
where
    S: Stream<Item = I> + Unpin,
    I: Clone + Unpin,
{
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.s).poll_next(cx) {
            Poll::Ready(Some(item)) => {
                self.items.push_back(item.clone());
                Poll::Ready(Some(item))
            }
            r @ _ => r,
        }
    }
}

impl<S, I> Stream for Cued<S>
where
    S: Stream<Item = I> + Unpin,
    I: Unpin,
{
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(item) = self.items.pop_front() {
            return Poll::Ready(Some(item));
        }

        Pin::new(&mut self.s).poll_next(cx)
    }
}
