//! RAM filesystem used by [ArceOS](https://github.com/arceos-org/arceos).
//!
//! The implementation is based on [`axfs_vfs`].

#[path = "dir.rs"]
mod dir;
#[path = "file.rs"]
mod file;

pub use self::dir::DirNode;
pub use self::file::FileNode;

use super::Mutex;
use alloc::sync::Arc;
use axfs_vfs::{VfsNodeRef, VfsOps, VfsResult};

/// A RAM filesystem that implements [`axfs_vfs::VfsOps`].
pub struct RamFileSystem {
    parent: Mutex<Option<VfsNodeRef>>,
    root: Arc<DirNode>,
}

impl RamFileSystem {
    /// Create a new instance.
    pub fn new() -> Self {
        Self {
            parent: Mutex::new(None),
            root: DirNode::new(None),
        }
    }

    /// Returns the root directory node in [`Arc<DirNode>`](DirNode).
    pub fn root_dir_node(&self) -> Arc<DirNode> {
        self.root.clone()
    }
}

impl VfsOps for RamFileSystem {
    fn mount(&self, _path: &str, mount_point: VfsNodeRef) -> VfsResult {
        let parent = mount_point.parent();
        self.root.set_parent(parent.as_ref());
        *self.parent.lock() = parent;
        Ok(())
    }

    fn root_dir(&self) -> VfsNodeRef {
        self.root.clone()
    }
}

impl Default for RamFileSystem {
    fn default() -> Self {
        Self::new()
    }
}
