use std::{
    io::{self, Read, Write},
    process::exit,
    thread,
};

use anyhow::{anyhow, Result};
use termion::raw::IntoRawMode;

use super::ControllerContext;

pub struct ControllerConsole<'a> {
    context: &'a mut ControllerContext,
}

impl ControllerConsole<'_> {
    pub fn new(context: &mut ControllerContext) -> ControllerConsole<'_> {
        ControllerConsole { context }
    }

    pub fn perform(&mut self, id: &str) -> Result<()> {
        let info = self
            .context
            .resolve(id)?
            .ok_or_else(|| anyhow!("unable to resolve container: {}", id))?;
        let domid = info.domid;
        let (mut read, mut write) = self.context.xen.open_console(domid)?;
        let mut stdin = io::stdin();
        let is_tty = termion::is_tty(&stdin);
        let mut stdout_for_exit = io::stdout().into_raw_mode()?;
        thread::spawn(move || {
            let mut buffer = vec![0u8; 60];
            loop {
                let size = stdin.read(&mut buffer).expect("failed to read stdin");
                if is_tty && size == 1 && buffer[0] == 0x1d {
                    stdout_for_exit
                        .suspend_raw_mode()
                        .expect("failed to disable raw mode");
                    stdout_for_exit.flush().expect("failed to flush stdout");
                    exit(0);
                }
                write
                    .write_all(&buffer[0..size])
                    .expect("failed to write to domain console");
                write.flush().expect("failed to flush domain console");
            }
        });

        let mut buffer = vec![0u8; 256];
        if is_tty {
            let mut stdout = io::stdout().into_raw_mode()?;
            loop {
                let size = read.read(&mut buffer)?;
                stdout.write_all(&buffer[0..size])?;
                stdout.flush()?;
            }
        } else {
            let mut stdout = io::stdout();
            loop {
                let size = read.read(&mut buffer)?;
                stdout.write_all(&buffer[0..size])?;
                stdout.flush()?;
            }
        }
    }
}
