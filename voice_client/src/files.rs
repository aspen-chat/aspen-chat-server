//! Files as the engine sends and receives them, through interfaces the platform implements so
//! that a file's bytes never cross the platform boundary whole: a file the user picked to offer
//! is read a chunk at a time as each transfer sends it, and a file being received is written a
//! chunk at a time into the place the user chose before accepting it. The engine names both by
//! opaque references the platform gave the page (a picked file's, a destination's), and the
//! platform decides what each means (a security-scoped URL on iOS, a document URI on Android);
//! nothing here is a path the page could choose. Swift and Kotlin implement these through
//! `bindings`. Every method may be called from the engine's thread or a task of its runtime.

use async_trait::async_trait;
use std::sync::Arc;

/// What the platform could not do with a file, in its own words: it is gone, cannot be read, the
/// place it goes cannot be written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct FileError(pub String);

/// A file the user offered, opened for one transfer of it (or for its size alone), read from its
/// start in order. Each transfer of an offer opens the file again, so two receivers read it at
/// their own pace.
#[async_trait]
pub trait FileSource: Send + Sync {
    /// The file's size in bytes as the platform has it when opened, which is what the offer
    /// states and what a transfer of it must send.
    fn size(&self) -> u64;
    /// The next bytes, at most `max_bytes` of them; empty once the file has ended.
    async fn read(&self, max_bytes: u64) -> Result<Vec<u8>, FileError>;
    /// Done reading. Closing a closed source does nothing.
    fn close(&self);
}

/// A file being received, in the place its receiver chose; written in order.
#[async_trait]
pub trait FileSink: Send + Sync {
    async fn write(&self, data: Vec<u8>) -> Result<(), FileError>;
    /// Every byte is in: keeps the file.
    async fn finish(&self) -> Result<(), FileError>;
    /// The transfer did not finish: removes what was written, since the file holds only part of
    /// what was sent. Discarding a finished or discarded sink does nothing.
    async fn discard(&self);
}

/// The platform's files, by the references it gave the page.
#[async_trait]
pub trait Files: Send + Sync {
    /// Opens the picked file `reference` names for reading from its start.
    async fn open(&self, reference: String) -> Result<Arc<dyn FileSource>, FileError>;
    /// The engine is done with the picked file `reference` names: its offer is gone and nothing
    /// sends it, so the platform may let the file go.
    fn release(&self, reference: String);
    /// Opens the place `destination` names, which the user chose for a file not yet accepted,
    /// for writing from its start.
    async fn create_sink(&self, destination: String) -> Result<Arc<dyn FileSink>, FileError>;
}
