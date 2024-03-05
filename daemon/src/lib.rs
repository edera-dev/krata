use anyhow::Result;
use handlers::{
    console::ConsoleStreamRequestHandler, destroy::DestroyRequestHandler,
    launch::LaunchRequestHandler, list::ListRequestHandler,
};
use listen::{DaemonListener, DaemonRequestHandlers};
use runtime::Runtime;
use tokio_listener::Listener;

pub mod handlers;
pub mod listen;
pub mod runtime;

pub struct Daemon {
    runtime: Runtime,
}

impl Daemon {
    pub async fn new(runtime: Runtime) -> Result<Self> {
        Ok(Self { runtime })
    }

    pub async fn listen(&mut self, listener: Listener) -> Result<()> {
        let handlers = DaemonRequestHandlers::new(
            self.runtime.clone(),
            vec![
                Box::new(LaunchRequestHandler::new()),
                Box::new(DestroyRequestHandler::new()),
                Box::new(ConsoleStreamRequestHandler::new()),
                Box::new(ListRequestHandler::new()),
            ],
        );
        let mut listener = DaemonListener::new(listener, handlers);
        listener.handle().await?;
        Ok(())
    }
}
