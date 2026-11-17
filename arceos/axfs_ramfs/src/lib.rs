//! RAM filesystem used by ArceOS, based on axfs_vfs.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use spin::Mutex;

mod ramfs;
pub use ramfs::{DirNode, FileNode, RamFileSystem};

#[cfg(test)]
mod tests;
