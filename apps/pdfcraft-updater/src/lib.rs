//! PdfCraft Updater: the small app that Help ▸ Check for updates starts for Merge and update and
//! for Install and relaunch. It runs on its own (closing PdfCraft leaves it running, and it can be
//! minimized) and shows each stage of the update: the fork's Sync upstream run, its pull request
//! and every check on it, the merge, the Release build, the published release, then the download,
//! the checks on it, the install and the relaunch. A stage that fails says why and links to the
//! page that shows it.
//!
//! GitHub is read with the GitHub CLI (`gh api`) and the user's own sign-in; the install itself is
//! the installer inside PdfCraft.app (packaging/macos/install.sh), whose `==> ` lines are its stages.

pub mod args;
pub mod gh;
pub mod install;
pub mod stages;
pub mod view;
pub mod watch;

/// The updater's own lock beside the installers' (`gh::INSTALLER_LOCK`): a symbolic link to its
/// process id, so PdfCraft can tell an update is under way.
pub const UPDATER_LOCK: &str = "pdfcraft-updater.lock";

/// The helper app's name inside `PdfCraft.app/Contents/Helpers`.
pub const HELPER_APP: &str = "PdfCraft Updater.app";
