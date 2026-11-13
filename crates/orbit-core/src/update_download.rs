use crate::error::{Result, fail};
use std::{collections::VecDeque, future::Future, sync::Mutex, time::Duration};
use tokio::sync::watch;

#[derive(Default)]
pub struct Downloads {
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    active: Option<(String, watch::Sender<bool>)>,
    // A cancel IPC can reach native code before the corresponding start IPC.
    // Bound these receipts; every attempt uses a new UUID.
    cancelled: VecDeque<String>,
}
pub struct Download<'a> {
    owner: &'a Downloads,
    id: String,
    cancelled: watch::Receiver<bool>,
}
fn validate_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id).map_err(|_| fail("invalid_update_request"))?;
    Ok(())
}
impl Downloads {
    pub fn begin(&self, id: &str) -> Result<Download<'_>> {
        validate_id(id)?;
        let mut state = self.state.lock().map_err(|_| fail("state_unavailable"))?;
        if let Some(index) = state.cancelled.iter().position(|receipt| receipt == id) {
            state.cancelled.remove(index);
            return Err(fail("update_download_cancelled"));
        }
        if state.active.is_some() {
            return Err(fail("update_busy"));
        }
        let (sender, cancelled) = watch::channel(false);
        state.active = Some((id.to_owned(), sender));
        Ok(Download {
            owner: self,
            id: id.to_owned(),
            cancelled,
        })
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        let mut state = self.state.lock().map_err(|_| fail("state_unavailable"))?;
        if let Some((active, sender)) = &state.active
            && active == id
        {
            let _ = sender.send(true);
        } else if !state.cancelled.iter().any(|receipt| receipt == id) {
            if state.cancelled.len() == 16 {
                state.cancelled.pop_front();
            }
            state.cancelled.push_back(id.to_owned());
        }
        Ok(())
    }
}
impl Download<'_> {
    pub async fn run<T>(
        &self,
        work: impl Future<Output = Result<T>>,
        timeout: Duration,
    ) -> Result<T> {
        let mut cancellation = self.cancelled.clone();
        tokio::select! {
            biased;
            _ = cancellation.wait_for(|cancelled| *cancelled) => Err(fail("update_download_cancelled")),
            result = tokio::time::timeout(timeout, work) => {
                if *self.cancelled.borrow() {
                    return Err(fail("update_download_cancelled"));
                }
                result.map_err(|_| fail("update_request_timeout"))?
            }
        }
    }
}
impl Drop for Download<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.owner.state.lock()
            && state.active.as_ref().is_some_and(|(id, _)| id == &self.id)
        {
            state.active = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::uuid;

    #[tokio::test]
    async fn cancellation_drops_in_flight_work_and_releases_the_slot() {
        let downloads = Downloads::default();
        let id = uuid();
        let download = downloads.begin(&id).unwrap();
        assert!(downloads.begin(&uuid()).is_err());
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<()>(1);
        let work = async move {
            let _sender = sender;
            std::future::pending::<Result<()>>().await
        };
        let result = async {
            downloads.cancel(&uuid()).unwrap();
            assert!(!*download.cancelled.borrow());
            downloads.cancel(&id).unwrap();
            download.run(work, Duration::from_secs(1)).await
        }
        .await;
        assert_eq!(result.unwrap_err().code, "update_download_cancelled");
        assert_eq!(receiver.recv().await, None);
        drop(download);
        assert!(downloads.begin(&uuid()).is_ok());
    }

    #[tokio::test]
    async fn early_cancel_and_timeout_leave_retry_available() {
        let downloads = Downloads::default();
        let id = uuid();
        downloads.cancel(&id).unwrap();
        assert!(
            matches!(downloads.begin(&id), Err(error) if error.code == "update_download_cancelled")
        );
        let download = downloads.begin(&uuid()).unwrap();
        let result = download
            .run(
                std::future::pending::<Result<()>>(),
                Duration::from_millis(5),
            )
            .await;
        assert_eq!(result.unwrap_err().code, "update_request_timeout");
        drop(download);
        assert!(downloads.begin(&uuid()).is_ok());
    }
}
