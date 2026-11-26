use anyhow::{Context, Result};
use futures::channel::mpsc::{self as futures_mpsc, UnboundedReceiver};
use kanso_core::WorkspaceConfig;
use kanso_engine::{EngineCommand, EngineConfig, EngineEvent, EngineHandle};
use std::thread::JoinHandle;

pub struct EngineLink {
    handle: Option<EngineHandle>,
    bridge_thread: Option<JoinHandle<()>>,
}

impl EngineLink {
    pub fn start(ws: WorkspaceConfig) -> Result<(Self, UnboundedReceiver<EngineEvent>)> {
        let mut handle = kanso_engine::start(EngineConfig::new(ws))?;
        let std_rx = handle
            .take_events()
            .context("新启动引擎未返回事件接收通道")?;
        let (tx, rx) = futures_mpsc::unbounded();

        let bridge = std::thread::Builder::new()
            .name("kanso-engine-ui-bridge".to_string())
            .spawn(move || {
                while let Ok(ev) = std_rx.recv() {
                    if tx.unbounded_send(ev).is_err() {
                        break;
                    }
                }
            })?;

        Ok((
            Self {
                handle: Some(handle),
                bridge_thread: Some(bridge),
            },
            rx,
        ))
    }

    pub fn send(&self, cmd: EngineCommand) -> Result<()> {
        if let Some(handle) = &self.handle {
            handle.send(cmd)?;
        }
        Ok(())
    }

    pub fn shutdown(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.shutdown();
        }
        if let Some(bridge) = self.bridge_thread.take() {
            let _ = bridge.join();
        }
    }
}

impl Drop for EngineLink {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.shutdown();
        }
        if let Some(bridge) = self.bridge_thread.take() {
            let _ = bridge.join();
        }
    }
}
