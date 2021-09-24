use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::PathBuf;

use anyhow::Result;
use tokio::fs::{File, OpenOptions};
use tokio::io::{stdin, stdout};

pub async fn path_to_async_read(p: Option<PathBuf>) -> Result<File> {
    match p {
        Some(p) => {
            if p.to_str() == Some("-") {
                unsafe { Ok(File::from_raw_fd(stdin().as_raw_fd())) }
            } else {
                Ok(OpenOptions::new().read(true).open(p).await?)
            }
        }
        None => unsafe { Ok(File::from_raw_fd(stdin().as_raw_fd())) },
    }
}

pub async fn path_to_async_write(p: Option<PathBuf>) -> Result<File> {
    match p {
        Some(p) => {
            if p.to_str() == Some("-") {
                unsafe { Ok(File::from_raw_fd(stdout().as_raw_fd())) }
            } else {
                Ok(OpenOptions::new().write(true).create(true).open(p).await?)
            }
        }
        None => unsafe { Ok(File::from_raw_fd(stdout().as_raw_fd())) },
    }
}
