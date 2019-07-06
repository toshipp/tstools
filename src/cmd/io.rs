use std::io::Result;
use std::io::{Read, Write};
use std::path::PathBuf;

use failure::Error;
use tokio::fs::OpenOptions;
use tokio::io::{stdin, stdout};
use tokio::prelude::future::ok;
use tokio::prelude::future::Either;
use tokio::prelude::{Async, Future};
use tokio::prelude::{AsyncRead, AsyncWrite};

enum EitherIO<A, B> {
    A(A),
    B(B),
}

impl<A, B> Read for EitherIO<A, B>
where
    A: Read,
    B: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self {
            EitherIO::A(a) => a.read(buf),
            EitherIO::B(b) => b.read(buf),
        }
    }
}

impl<A, B> Write for EitherIO<A, B>
where
    A: Write,
    B: Write,
{
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        match self {
            EitherIO::A(a) => a.write(buf),
            EitherIO::B(b) => b.write(buf),
        }
    }

    fn flush(&mut self) -> Result<()> {
        match self {
            EitherIO::A(a) => a.flush(),
            EitherIO::B(b) => b.flush(),
        }
    }
}

impl<A, B> AsyncWrite for EitherIO<A, B>
where
    A: AsyncWrite,
    B: AsyncWrite,
{
    fn shutdown(&mut self) -> Result<Async<()>> {
        match self {
            EitherIO::A(a) => a.shutdown(),
            EitherIO::B(b) => b.shutdown(),
        }
    }
}

impl<A, B> AsyncRead for EitherIO<A, B>
where
    A: AsyncRead,
    B: AsyncRead,
{
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        match self {
            EitherIO::A(a) => a.prepare_uninitialized_buffer(buf),
            EitherIO::B(b) => b.prepare_uninitialized_buffer(buf),
        }
    }
}

pub fn path_to_async_read(p: Option<PathBuf>) -> impl Future<Item = impl AsyncRead, Error = Error> {
    match p {
        Some(p) => {
            if p.to_str() == Some("-") {
                Either::A(ok(EitherIO::A(stdin())))
            } else {
                let mut option = OpenOptions::new();
                Either::B(
                    option
                        .read(true)
                        .open(p)
                        .map(|file| EitherIO::B(file))
                        .map_err(|e| Error::from(e)),
                )
            }
        }
        None => Either::A(ok(EitherIO::A(stdin()))),
    }
}

pub fn path_to_async_write(
    p: Option<PathBuf>,
) -> impl Future<Item = impl AsyncWrite, Error = Error> {
    match p {
        Some(p) => {
            if p.to_str() == Some("-") {
                Either::A(ok(EitherIO::A(stdout())))
            } else {
                let mut option = OpenOptions::new();
                Either::B(
                    option
                        .write(true)
                        .create(true)
                        .open(p)
                        .map(|file| EitherIO::B(file))
                        .map_err(|e| Error::from(e)),
                )
            }
        }
        None => Either::A(ok(EitherIO::A(stdout()))),
    }
}
