//! Bounded SQLite statement waits, enabled only by blocking checkpoint workers.
use super::*;
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

struct Budget {
    deadline: Instant,
    cancel: Option<CancellationToken>,
    authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    revoked: Cell<bool>,
}
thread_local! {
    static BUDGET: RefCell<Option<Budget>> = const { RefCell::new(None) };
}

pub(in crate::attachment) struct Wait(Option<Budget>);
impl Wait {
    pub(in crate::attachment) fn new(
        cancel: Option<CancellationToken>,
        authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    ) -> Self {
        Self(BUDGET.replace(Some(Budget {
            deadline: Instant::now() + Duration::from_secs(2),
            cancel,
            authority,
            revoked: Cell::new(false),
        })))
    }

    pub(in crate::attachment) fn revoked(&self) -> bool {
        BUDGET.with_borrow(|b| b.as_ref().is_some_and(|b| b.revoked.get()))
    }
}
impl Drop for Wait {
    fn drop(&mut self) {
        BUDGET.replace(self.0.take());
    }
}

fn wait_for_lock(_: i32) -> bool {
    BUDGET.with_borrow(|budget| {
        let Some(budget) = budget else { return false };
        if budget
            .authority
            .as_ref()
            .is_some_and(|a| a.check().is_err())
        {
            budget.revoked.set(true);
            return false;
        }
        if budget
            .cancel
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return false;
        }
        let remaining = budget.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
        if budget
            .authority
            .as_ref()
            .is_some_and(|a| a.check().is_err())
        {
            budget.revoked.set(true);
            return false;
        }
        Instant::now() < budget.deadline
            && !budget
                .cancel
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
    })
}

impl Journal {
    pub(in crate::attachment) fn begin_checkpoint_wait(&self) -> Result<()> {
        self.connection.busy_handler(Some(wait_for_lock))?;
        Ok(())
    }

    pub(in crate::attachment) fn end_checkpoint_wait(&self) -> Result<()> {
        self.connection.busy_timeout(Duration::ZERO)?;
        Ok(())
    }
}
