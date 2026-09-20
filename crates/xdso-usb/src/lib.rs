//! talking to the actual scope over usb.
//!
//! this is the only crate that knows usb exists. it hands you a [`Scope`] with
//! one method per command, and a [`Poller`] that drives one on its own thread
//! so the ui never blocks on a 980 ms screenshot.
//!
//! pure rust usb via nusb, no libusb, so theres nothing to install and nothing
//! to ship alongside the binary.

pub mod device;
pub mod poller;

pub use device::{
    CMD_GAP, Error, GAP_MAX, GAP_RELIABLE_MIN, MAX_COMMAND, Scope, ScopeTime, Screen,
};
pub use poller::{Command, Feed, Poller, ShellReply, Snapshot, Tuning};
