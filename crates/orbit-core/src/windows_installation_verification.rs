//! Share Windows signature checks only while a synchronous transaction holds
//! the verified installation open against writes and replacement.
use crate::{dto::ClientKind, error::Result};
use std::{cell::RefCell, fs::File, marker::PhantomData, rc::Rc};

struct PinnedInstallation {
    client: ClientKind,
    path: String,
    app_version: String,
    cli_version: Option<String>,
    _handles: Vec<File>,
}

enum Verification {
    Inactive,
    Active(Option<PinnedInstallation>),
}

thread_local! {
    static VERIFICATION: RefCell<Verification> = const { RefCell::new(Verification::Inactive) };
}

pub(crate) struct InstallationVerificationScope {
    outermost: bool,
    // The handles belong to this thread. A scope cannot cross an async task boundary.
    _thread: PhantomData<Rc<()>>,
}

impl InstallationVerificationScope {
    pub(crate) fn enter() -> Self {
        let outermost = VERIFICATION.with_borrow_mut(|scope| match scope {
            Verification::Active(_) => false,
            Verification::Inactive => {
                *scope = Verification::Active(None);
                true
            }
        });
        Self {
            outermost,
            _thread: PhantomData,
        }
    }

    pub(super) fn verify(
        client: ClientKind,
        path: &str,
        app_version: &str,
        cli_version: Option<&str>,
        verify_installation: impl FnOnce() -> Result<Vec<File>>,
    ) -> Result<()> {
        if VERIFICATION.with_borrow(|state| {
            matches!(state, Verification::Active(Some(p))
            if p.client == client && p.path == path && p.app_version == app_version && p.cli_version.as_deref() == cli_version)
        }) {
            return Ok(());
        }
        let handles = verify_installation()?;
        VERIFICATION.with_borrow_mut(|state| {
            if let Verification::Active(slot) = state {
                *slot = Some(PinnedInstallation {
                    client,
                    path: path.into(),
                    app_version: app_version.into(),
                    cli_version: cli_version.map(str::to_owned),
                    _handles: handles,
                });
            }
        });
        Ok(())
    }
}
impl Drop for InstallationVerificationScope {
    fn drop(&mut self) {
        if self.outermost {
            VERIFICATION.with_borrow_mut(|state| *state = Verification::Inactive);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::fail;
    use std::cell::Cell;

    #[test]
    fn claude_pins_only_its_own_verified_installation_and_rechecks_the_next_operation() {
        let calls = Cell::new(0);
        let verify = |client| {
            InstallationVerificationScope::verify(client, "same path", "1", None, || {
                calls.set(calls.get() + 1);
                Ok(vec![tempfile::tempfile().unwrap()])
            })
            .unwrap()
        };
        {
            let _scope = InstallationVerificationScope::enter();
            verify(ClientKind::Claude);
            verify(ClientKind::Claude);
            verify(ClientKind::Codex);
            verify(ClientKind::Claude);
            assert_eq!(calls.get(), 3);
        }
        let _scope = InstallationVerificationScope::enter();
        verify(ClientKind::Claude);
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn codex_reuses_only_successful_pinned_checks_for_the_same_target_within_one_operation() {
        let calls = Cell::new(0);
        let verify = |path, app, cli| {
            InstallationVerificationScope::verify(ClientKind::Codex, path, app, Some(cli), || {
                calls.set(calls.get() + 1);
                Ok(vec![tempfile::tempfile().unwrap()])
            })
            .unwrap()
        };
        {
            let _review = InstallationVerificationScope::enter();
            assert!(
                InstallationVerificationScope::verify(
                    ClientKind::Codex,
                    "A",
                    "1",
                    Some("1"),
                    || Err(fail("invalid_signature"))
                )
                .is_err()
            );
            verify("A", "1", "1");
            {
                let _nested = InstallationVerificationScope::enter();
                for _ in 0..26 {
                    verify("A", "1", "1");
                }
            }
            verify("A", "1", "1");
            assert_eq!(calls.get(), 1);
            verify("B", "1", "1");
            verify("B", "2", "1");
            verify("B", "2", "2");
            assert_eq!(calls.get(), 4);
        }
        // Standalone preflights and the subsequent confirm each start fresh.
        verify("B", "2", "2");
        verify("B", "2", "2");
        let _confirmation = InstallationVerificationScope::enter();
        verify("B", "2", "2");
        assert_eq!(calls.get(), 7);
    }
}
