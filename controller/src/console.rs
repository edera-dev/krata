use std::{
    io::{stdin, stdout},
    os::fd::{AsRawFd, FromRawFd},
};

use anyhow::Result;
use futures::future::join_all;
use log::warn;
use std::process::exit;
use termion::raw::IntoRawMode;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
};

pub struct XenConsole {
    xen_read_handle: File,
    xen_write_handle: File,
}

impl XenConsole {
    pub async fn new(tty: &str) -> Result<XenConsole> {
        let xen_read_handle = File::options().read(true).write(false).open(tty).await?;
        let xen_write_handle = File::options().read(false).write(true).open(tty).await?;
        Ok(XenConsole {
            xen_read_handle,
            xen_write_handle,
        })
    }

    pub async fn attach(self) -> Result<()> {
        let stdin = stdin();
        let terminal = stdout().into_raw_mode()?;
        let stdout = unsafe { File::from_raw_fd(terminal.as_raw_fd()) };
        let reader_task = tokio::task::spawn(async move {
            if let Err(error) = XenConsole::copy_stdout(stdout, self.xen_read_handle).await {
                warn!("failed to copy console output: {}", error);
            }
        });
        let writer_task = tokio::task::spawn(async move {
            if let Err(error) = XenConsole::intercept_stdin(
                unsafe { File::from_raw_fd(stdin.as_raw_fd()) },
                self.xen_write_handle,
            )
            .await
            {
                warn!("failed to intercept stdin: {}", error);
            }
        });

        join_all(vec![reader_task, writer_task]).await;
        Ok(())
    }

    async fn copy_stdout(mut stdout: File, mut console: File) -> Result<()> {
        let mut buffer = vec![0u8; 256];
        loop {
            let size = console.read(&mut buffer).await?;
            stdout.write_all(&buffer[0..size]).await?;
            stdout.flush().await?;
        }
    }

    async fn intercept_stdin(mut stdin: File, mut console: File) -> Result<()> {
        let mut buffer = vec![0u8; 60];
        loop {
            let size = stdin.read(&mut buffer).await?;
            if size == 1 && buffer[0] == 0x1d {
                exit(0);
            }
            console.write_all(&buffer[0..size]).await?;
        }
    }
}
