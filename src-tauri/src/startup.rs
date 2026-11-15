//! Windows startup readiness. Native I/O and package probes never hold the UI loop.
use orbit_core::error::{Result, fail};

pub(crate) struct Startup(tokio::sync::watch::Sender<Option<Result<()>>>);
impl Default for Startup {
    fn default() -> Self {
        Self(tokio::sync::watch::channel(None).0)
    }
}
impl Startup {
    pub(crate) fn finish(&self, result: Result<()>) {
        self.0.send_replace(Some(result));
    }
    pub(crate) async fn wait(&self) -> Result<()> {
        let mut ready = self.0.subscribe();
        loop {
            if let Some(result) = ready.borrow_and_update().clone() {
                return result;
            }
            ready
                .changed()
                .await
                .map_err(|_| fail("state_unavailable"))?;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn all_waiters_receive_initialization_success_or_failure() {
        for result in [Ok(()), Err(fail("vault_unavailable"))] {
            let state = std::sync::Arc::new(Startup::default());
            let a = {
                let state = state.clone();
                tokio::spawn(async move { state.wait().await })
            };
            let b = {
                let state = state.clone();
                tokio::spawn(async move { state.wait().await })
            };
            tokio::task::yield_now().await;
            assert!(!a.is_finished() && !b.is_finished());
            state.finish(result.clone());
            assert_eq!(a.await.unwrap(), result);
            assert_eq!(b.await.unwrap(), result);
            assert_eq!(state.wait().await, result);
        }
    }
}
