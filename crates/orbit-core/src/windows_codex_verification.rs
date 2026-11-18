//! Share Windows signature checks only while a synchronous transaction holds
//! the verified installation open against writes and replacement.
use crate::error::Result;
use std::{cell::RefCell, fs::File, marker::PhantomData, rc::Rc};

struct PinnedInstallation {
    path: String,
    app_version: String,
    cli_version: String,
    _handles: Vec<File>,
}

enum Verification {
    Inactive,
    Active(Option<PinnedInstallation>),
}

thread_local! {
    static VERIFICATION: RefCell<Verification> = const { RefCell::new(Verification::Inactive) };
}

pub(crate) struct CodexVerificationScope {
    outermost: bool,
    // The handles belong to this thread. A scope cannot cross an async task boundary.
    _thread: PhantomData<Rc<()>>,
}

impl CodexVerificationScope {
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
        path: &str,
        app_version: &str,
        cli_version: &str,
        verify_installation: impl FnOnce() -> Result<Vec<File>>,
    ) -> Result<()> {
        if VERIFICATION.with_borrow(|state| {
            matches!(state, Verification::Active(Some(p))
            if p.path == path && p.app_version == app_version && p.cli_version == cli_version)
        }) {
            return Ok(());
        }
        let handles = verify_installation()?;
        VERIFICATION.with_borrow_mut(|state| {
            if let Verification::Active(slot) = state {
                *slot = Some(PinnedInstallation {
                    path: path.into(),
                    app_version: app_version.into(),
                    cli_version: cli_version.into(),
                    _handles: handles,
                });
            }
        });
        Ok(())
    }
}
impl Drop for CodexVerificationScope {
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
    fn codex_reuses_only_successful_pinned_checks_for_the_same_target_within_one_operation() {
        let calls = Cell::new(0);
        let verify = |path, app, cli| {
            CodexVerificationScope::verify(path, app, cli, || {
                calls.set(calls.get() + 1);
                Ok(vec![tempfile::tempfile().unwrap()])
            })
            .unwrap()
        };
        {
            let _review = CodexVerificationScope::enter();
            assert!(
                CodexVerificationScope::verify("A", "1", "1", || Err(fail("invalid_signature")))
                    .is_err()
            );
            verify("A", "1", "1");
            {
                let _nested = CodexVerificationScope::enter();
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
        let _confirmation = CodexVerificationScope::enter();
        verify("B", "2", "2");
        assert_eq!(calls.get(), 7);
    }
}
