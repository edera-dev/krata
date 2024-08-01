// TODO: DELETE THESE
#![allow(unused_imports)]
#![allow(unused_labels)]

use std::{os::raw::c_int, time::Duration};

use anyhow::Result;
use tokio::{
  sync::{MutexGuard, Notify},
  time::sleep,
};
use xenstore::{XsdClient, XsdInterface};

pub mod background;
pub mod childwait;
pub mod exec;
pub mod init;
pub mod metrics;
pub mod spawn;
// pub mod supervisor;

pub async fn death(code: c_int) -> Result<()> {
  let store = XsdClient::open().await?;
  store
    .write_string("krata/guest/exit-code", &code.to_string())
    .await?;
  drop(store);
  loop {
    sleep(Duration::from_secs(1)).await;
  }
}

#[derive(Debug, Default)]
pub struct AsyncCondvar {
  inner: Notify
}

impl AsyncCondvar {
  pub fn new() -> Self { Self::default() }

  pub async fn wait<'a, T>(&self, lock: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    let fut = self.inner.notified();
    tokio::pin!(fut);
    fut.as_mut().enable();

    let mutex = MutexGuard::mutex(&lock);
    drop(lock);

    fut.await;
    mutex.lock().await
  }

  pub fn signal(&self) {
    self.inner.notify_waiters()
  }
}
