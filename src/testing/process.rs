//! A fake launcher that records the command and hands out a [`FakeCdp`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use futures::future::BoxFuture;

use crate::domain::error::Result;
use crate::ports::{BrowserLauncher, BrowserProcess, LaunchCommand, Launched};
use crate::testing::fake_cdp::FakeCdp;

#[derive(Clone, Default)]
pub struct FakeLauncher {
    pub cdp: FakeCdp,
    pub commands: Arc<Mutex<Vec<LaunchCommand>>>,
    pub shut_down: Arc<AtomicBool>,
}

struct FakeProcess(Arc<AtomicBool>);

impl BrowserProcess for FakeProcess {
    fn pid(&self) -> Option<u32> {
        Some(42)
    }
    fn shutdown(&self) -> BoxFuture<'static, ()> {
        self.0.store(true, Ordering::SeqCst);
        futures::future::ready(()).boxed()
    }
}

impl BrowserLauncher for FakeLauncher {
    fn launch(&self, command: LaunchCommand) -> BoxFuture<'static, Result<Launched>> {
        self.commands.lock().unwrap().push(command);
        let launched = Launched {
            cdp: Arc::new(self.cdp.clone()),
            process: Box::new(FakeProcess(self.shut_down.clone())),
        };
        futures::future::ready(Ok(launched)).boxed()
    }
}
