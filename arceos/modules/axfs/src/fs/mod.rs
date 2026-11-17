cfg_if::cfg_if! {
    if #[cfg(feature = "myfs")] {
        pub mod myfs;
    } else if #[cfg(feature = "fatfs")] {
        pub mod fatfs;
    }
}

#[cfg(feature = "devfs")]
pub use axfs_devfs as devfs;

// Compile the local RAMFS source with this module's existing kernel mutex.
// This keeps the teaching implementation in use without changing Cargo files.
#[cfg(any(feature = "ramfs", feature = "procfs", feature = "sysfs"))]
use axsync::Mutex;
#[cfg(any(feature = "ramfs", feature = "procfs", feature = "sysfs"))]
#[path = "../../../../axfs_ramfs/src/ramfs.rs"]
pub mod ramfs;
