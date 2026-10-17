use super::{Action, Error, Protection, Report, Request};
use std::{borrow::Borrow, collections::HashSet};

/// Local asynchronous ticket. Its UUID and epoch do not come from the helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub operation_id: String,
    pub generation: u64,
    pub request: Request,
}

/// In-memory evidence only; never deserialize a saved verified UI state.
#[derive(Debug, Default)]
pub struct State {
    pub busy: bool,
    /// Local input/operation epoch, unrelated to helper Unix milliseconds.
    pub generation: u64,
    pub last_report: Option<Report>,
    pub last_error: Option<Error>,
    active: Option<Operation>,
    service_generation: Option<u64>,
    consumed: HashSet<String>,
    epoch_exhausted: bool,
}

impl State {
    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn report(&self) -> Option<&Report> {
        self.last_report.as_ref()
    }

    pub fn service_generation(&self) -> Option<u64> {
        self.service_generation
    }

    pub fn protection(&self) -> Protection {
        self.last_report
            .as_ref()
            .filter(|_| !self.busy)
            .map_or(Protection::Blocked, Report::effective_protection)
    }

    pub fn tailscale_connected(&self) -> bool {
        !self.busy
            && self
                .last_report
                .as_ref()
                .is_some_and(Report::tailscale_connected)
    }

    pub fn german_exit_verified(&self) -> bool {
        !self.busy
            && self
                .last_report
                .as_ref()
                .is_some_and(Report::german_exit_verified)
    }

    pub fn functional_ready(&self) -> bool {
        !self.busy
            && self
                .last_report
                .as_ref()
                .is_some_and(Report::functional_ready)
    }

    /// Clears all old evidence even if validation fails. Busy operations cannot
    /// overlap. An edit invalidates their evidence but does not cancel a helper
    /// that might still be mutating the host.
    pub fn begin(&mut self, request: impl Borrow<Request>) -> Result<Operation, Error> {
        if self.active.is_some() || self.busy {
            return Err(Error::Busy);
        }
        self.invalidate()?;
        let request = request.borrow();
        if let Err(error) = request.validate() {
            self.last_error = Some(error);
            return Err(error);
        }
        let ticket = Operation {
            operation_id: uuid::Uuid::new_v4().to_string(),
            generation: self.generation,
            request: request.clone(),
        };
        self.active = Some(ticket.clone());
        self.busy = true;
        Ok(ticket)
    }

    pub fn invalidate(&mut self) -> Result<(), Error> {
        self.last_report = None;
        self.last_error = None;
        if self.epoch_exhausted {
            self.last_error = Some(Error::GenerationExhausted);
            return Err(Error::GenerationExhausted);
        }
        self.generation = self.generation.checked_add(1).ok_or_else(|| {
            self.epoch_exhausted = true;
            self.last_error = Some(Error::GenerationExhausted);
            Error::GenerationExhausted
        })?;
        Ok(())
    }

    pub fn apply(&mut self, ticket: &Operation, report: Report) -> Result<(), Error> {
        self.finish_ticket(ticket)?;
        let report = match report.prepare() {
            Ok(report) => report,
            Err(error) => return self.reject(error),
        };
        if report.action != ticket.request.action
            || !matches!(report.action, Action::Doctor | Action::Status)
                && report.backend != ticket.request.backend
        {
            return self.reject(Error::ReportMismatch);
        }
        if self.consumed.contains(&report.operation_id) {
            return self.reject(Error::ReplayedReport);
        }
        // Each operation ID is one-use even when its report is regressed.
        self.consumed.insert(report.operation_id.clone());
        if self
            .service_generation
            .is_some_and(|generation| report.generation < generation)
        {
            return self.reject(Error::GenerationRegressed);
        }
        self.service_generation = Some(report.generation);
        self.last_error = None;
        self.last_report = Some(report);
        Ok(())
    }

    /// A successful read-only inspection is the explicit operator recovery
    /// event for the desktop mutation interlock. It never proves that a timed
    /// out mutation completed, so the caller must require a structured report
    /// whose phase says no mutation is active.
    pub fn inspection_is_settled(&self) -> bool {
        self.last_error.is_none()
            && self.last_report.as_ref().is_some_and(|report| {
                report.action == Action::Status
                    && report.outcome == super::Outcome::Succeeded
                    && matches!(
                        report.phase.as_str(),
                        "uninstalled" | "restored" | "installed_closed" | "connected_unverified"
                    )
            })
    }

    pub fn fail(&mut self, ticket: &Operation, error: Error) -> Result<(), Error> {
        self.finish_ticket(ticket)?;
        self.last_report = None;
        self.last_error = Some(error);
        Ok(())
    }

    fn reject(&mut self, error: Error) -> Result<(), Error> {
        self.last_report = None;
        self.last_error = Some(error);
        Err(error)
    }

    fn finish_ticket(&mut self, ticket: &Operation) -> Result<(), Error> {
        if self.active.as_ref() != Some(ticket) {
            // A delayed callback must not clear a newer operation's busy flag.
            return Err(Error::StaleOperation);
        }
        self.active = None;
        self.busy = false;
        if self.epoch_exhausted || ticket.generation != self.generation {
            return self.reject(Error::StaleOperation);
        }
        Ok(())
    }
}
