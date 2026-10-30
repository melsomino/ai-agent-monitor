//! Root lifetime: runtime, actor handles, and all startup-owned resources.
use crate::actors::monitor::{Command, Handle, State};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};
pub struct ActorSystem {
    runtime: Option<tokio::runtime::Runtime>,
    pub monitor: Handle,
}
impl ActorSystem {
    #[cfg(target_os = "macos")]
    pub fn start_notifications(
        &self,
        ui: crate::macos::bridge::Sender,
    ) -> mpsc::Sender<crate::macos::native::NotificationInput> {
        let (tx, rx) = mpsc::channel(128);
        self.runtime
            .as_ref()
            .unwrap()
            .spawn(crate::actors::human::notifications::run(
                self.monitor.clone(),
                rx,
                ui,
            ));
        tx
    }
    pub fn start_ipc(&self) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        crate::app::ipc::start(
            self.monitor.clone(),
            self.runtime.as_ref().unwrap().handle(),
        )
    }
    pub fn start(ui: crate::macos::bridge::Sender) -> anyhow::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let (tx, rx) = mpsc::channel(256);
        let (state_tx, state_rx) = watch::channel(Arc::new(State::default()));
        let handle = Handle {
            tx,
            state: state_rx,
        };
        let mut agent_ports = HashMap::new();
        for provider in crate::agents::sessions::adapters() {
            let source = provider.id();
            let (tx, rx) = mpsc::channel(64);
            agent_ports.insert(source, tx);
            runtime.spawn(crate::actors::agents::supervise(
                provider,
                rx,
                handle.clone(),
            ));
        }
        let (telegram_tx, telegram_rx) = mpsc::channel(128);
        let monitor_task = runtime.spawn(crate::actors::monitor::run(
            rx,
            state_tx,
            agent_ports,
            telegram_tx.clone(),
            ui.clone(),
        ));
        let stopped_ui = ui.clone();
        runtime.spawn(async move {
            if let Err(error) = monitor_task.await {
                eprintln!("Monitor actor stopped: {error}");
            }
            let _ = stopped_ui.send(crate::macos::bridge::Event::Stop).await;
        });
        runtime.spawn(crate::actors::human::telegram::run(
            telegram_rx,
            handle.clone(),
            telegram_tx,
        ));
        Ok(Self {
            runtime: Some(runtime),
            monitor: handle,
        })
    }
}
impl Drop for ActorSystem {
    fn drop(&mut self) {
        let _ = self.monitor.send(Command::Shutdown);
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(Duration::from_secs(3));
        }
    }
}
