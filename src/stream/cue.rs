use std::collections::VecDeque;
use tokio::prelude::{Async, Stream};

pub struct Cueable<S>
where
    S: Stream,
{
    inner: S,
    items: VecDeque<S::Item>,
}

pub struct Cued<S>
where
    S: Stream,
{
    inner: S,
    items: VecDeque<S::Item>,
}

pub fn cueable<S: Stream>(s: S) -> Cueable<S> {
    Cueable {
        inner: s,
        items: VecDeque::new(),
    }
}

impl<S> Cueable<S>
where
    S: Stream,
{
    pub fn cue(self) -> Cued<S> {
        Cued {
            inner: self.inner,
            items: self.items,
        }
    }
}

impl<S, I> Stream for Cueable<S>
where
    S: Stream<Item = I>,
    I: Clone,
{
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Result<Async<Option<Self::Item>>, Self::Error> {
        match self.inner.poll() {
            Ok(Async::Ready(Some(item))) => {
                self.items.push_back(item.clone());
                Ok(Async::Ready(Some(item)))
            }
            r @ _ => r,
        }
    }
}

impl<S> Stream for Cued<S>
where
    S: Stream,
{
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Result<Async<Option<Self::Item>>, Self::Error> {
        if let Some(item) = self.items.pop_front() {
            return Ok(Async::Ready(Some(item)));
        }

        self.inner.poll()
    }
}
