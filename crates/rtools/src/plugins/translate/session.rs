//! Request revisions prevent late network events from replacing newer edits.
use super::core::{Completion, TranslateError};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    Connecting,
    Streaming,
    Complete,
    Truncated,
    Stopped,
    Stale,
    Failed(TranslateError),
}

/// In-memory output survives launcher hiding, but is never written to disk.
#[derive(Default)]
pub struct Session {
    revision: u64,
    output_revision: Option<u64>,
    pub output: String,
    pub status: Status,
}

impl Session {
    /// Exchanges source and result, invalidating pending output without claiming a new translation.
    pub fn exchange(&mut self, source: String) -> String {
        let input = std::mem::replace(&mut self.output, source);
        self.invalidate();
        input
    }

    /// Starts a new generation while retaining the last result until content arrives.
    pub fn begin(&mut self) -> u64 {
        self.revision += 1;
        self.status = Status::Connecting;
        self.revision
    }

    /// Appends current content and discards late events from cancelled generations.
    pub fn append(&mut self, revision: u64, text: &str) -> bool {
        if revision != self.revision || !self.running() || text.is_empty() {
            return false;
        }
        if !self.output_is_current() {
            self.output.clear();
            self.output_revision = Some(revision);
        }
        self.output.push_str(text);
        self.status = Status::Streaming;
        true
    }

    /// Applies a terminal result only while its request still owns the session.
    pub fn finish(&mut self, revision: u64, result: Result<Completion, TranslateError>) {
        if revision != self.revision || !self.running() {
            return;
        }
        self.status = match result {
            Ok(Completion::Complete) => Status::Complete,
            Ok(Completion::Truncated) => Status::Truncated,
            Err(error) => Status::Failed(error),
        };
    }

    /// Cancels acceptance of chunks while preserving partial output.
    pub fn stop(&mut self) {
        if self.running() {
            self.status = Status::Stopped;
        }
    }

    /// Marks retained output as unrelated to the newly edited request.
    pub fn invalidate(&mut self) {
        self.revision += 1;
        self.status = if self.output.is_empty() {
            Status::Idle
        } else {
            Status::Stale
        };
    }

    /// Indicates whether this entity currently accepts streaming events.
    pub fn running(&self) -> bool {
        matches!(self.status, Status::Connecting | Status::Streaming)
    }

    /// Distinguishes the current request's partial output from a retained previous result.
    pub fn output_is_current(&self) -> bool {
        self.output_revision == Some(self.revision)
    }
}
