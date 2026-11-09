//! What differs between the desktop app and the browser build: files, file dialogs,
//! background work, timers and the window. The rest of the app goes through this module,
//! so it reads the same on both.

use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
mod desktop;
#[cfg(not(target_arch = "wasm32"))]
pub use desktop::*;

#[cfg(target_arch = "wasm32")]
pub mod web;
#[cfg(target_arch = "wasm32")]
pub use web::*;

/// File checks that go through [`fs`], since `Path::is_file` and friends always use the
/// real file system.
pub trait PathExt {
    fn vfs_is_file(&self) -> bool;
    fn vfs_is_dir(&self) -> bool;
    fn vfs_exists(&self) -> bool {
        self.vfs_is_file() || self.vfs_is_dir()
    }
}

impl PathExt for Path {
    fn vfs_is_file(&self) -> bool {
        fs::metadata(self).is_ok_and(|m| m.is_file())
    }
    fn vfs_is_dir(&self) -> bool {
        fs::metadata(self).is_ok_and(|m| m.is_dir())
    }
}
