use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::prelude::{Async, Stream};

pub struct Interrupter {
    interrupted: Arc<AtomicBool>,
}

pub struct Interrutible<S> {
    inner: S,
    interrupted: Arc<AtomicBool>,
}

pub fn interruptible<S>(s: S) -> (Interrutible<S>, Interrupter) {
    let flag = Arc::new(AtomicBool::new(false));
    (
        Interrutible {
            inner: s,
            interrupted: flag.clone(),
        },
        Interrupter { interrupted: flag },
    )
}

impl Interrupter {
    pub fn interrupt(&self) {
        self.interrupted.store(true, Ordering::Release)
    }
}

impl<S> Interrutible<S> {
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S> Stream for Interrutible<S>
where
    S: Stream,
{
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Result<Async<Option<Self::Item>>, Self::Error> {
        if self.interrupted.load(Ordering::Acquire) {
            return Ok(Async::Ready(None));
        }

        self.inner.poll()
    }
}
