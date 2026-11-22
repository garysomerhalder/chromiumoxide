//! Process signal port (Playwright's `handleSIGINT` / `handleSIGTERM` /
//! `handleSIGHUP`).

use futures::stream::BoxStream;

/// A signal the program can be asked to handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Ctrl-C.
    Interrupt,
    Terminate,
    Hangup,
}

/// Delivers process signals and ends the process.
pub trait Signals: Send + Sync {
    /// Every delivery of `signal` from now on. Listening replaces the
    /// signal's default action for the whole process. Empty where the
    /// platform has no such signal.
    fn listen(&self, signal: Signal) -> BoxStream<'static, ()>;
    /// Ends the process with `code`.
    fn exit(&self, code: i32) -> !;
}
