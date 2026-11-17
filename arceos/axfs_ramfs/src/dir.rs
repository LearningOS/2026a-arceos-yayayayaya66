use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::{string::String, vec::Vec};

use super::Mutex;
use axfs_vfs::{VfsDirEntry, VfsNodeAttr, VfsNodeOps, VfsNodeRef, VfsNodeType};
use axfs_vfs::{VfsError, VfsResult};

use super::file::FileNode;

/// The directory node in the RAM filesystem.
///
/// It implements [`axfs_vfs::VfsNodeOps`].
pub struct DirNode {
    this: Weak<DirNode>,
    parent: Mutex<Weak<dyn VfsNodeOps>>,
    children: Mutex<BTreeMap<String, VfsNodeRef>>,
}

impl DirNode {
    pub(super) fn new(parent: Option<Weak<dyn VfsNodeOps>>) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            this: this.clone(),
            parent: Mutex::new(parent.unwrap_or_else(|| Weak::<Self>::new())),
            children: Mutex::new(BTreeMap::new()),
        })
    }

    pub(super) fn set_parent(&self, parent: Option<&VfsNodeRef>) {
        *self.parent.lock() = parent.map_or(Weak::<Self>::new() as _, Arc::downgrade);
    }

    fn rename_parent<'a>(&self, path: &'a str) -> VfsResult<(VfsNodeRef, &'a str)> {
        let path = path.trim_start_matches('/');
        if path.is_empty() || path.ends_with('/') {
            return Err(VfsError::InvalidInput);
        }
        let (parent_path, name) = path.rsplit_once('/').unwrap_or(("", path));
        if name == "." || name == ".." {
            return Err(VfsError::InvalidInput);
        }
        let parent = self
            .this
            .upgrade()
            .ok_or(VfsError::NotFound)?
            .lookup(parent_path)?;
        Ok((parent, name))
    }

    /// Returns a string list of all entries in this directory.
    pub fn get_entries(&self) -> Vec<String> {
        self.children.lock().keys().cloned().collect()
    }

    /// Checks whether a node with the given name exists in this directory.
    pub fn exist(&self, name: &str) -> bool {
        self.children.lock().contains_key(name)
    }

    /// Creates a new node with the given name and type in this directory.
    pub fn create_node(&self, name: &str, ty: VfsNodeType) -> VfsResult {
        if self.exist(name) {
            log::error!("AlreadyExists {}", name);
            return Err(VfsError::AlreadyExists);
        }
        let node: VfsNodeRef = match ty {
            VfsNodeType::File => Arc::new(FileNode::new()),
            VfsNodeType::Dir => Self::new(Some(self.this.clone())),
            _ => return Err(VfsError::Unsupported),
        };
        self.children.lock().insert(name.into(), node);
        Ok(())
    }

    /// Removes a node by the given name in this directory.
    pub fn remove_node(&self, name: &str) -> VfsResult {
        let mut children = self.children.lock();
        let node = children.get(name).ok_or(VfsError::NotFound)?;
        if let Some(dir) = node.as_any().downcast_ref::<DirNode>() {
            if !dir.children.lock().is_empty() {
                return Err(VfsError::DirectoryNotEmpty);
            }
        }
        children.remove(name);
        Ok(())
    }
}

impl VfsNodeOps for DirNode {
    fn get_attr(&self) -> VfsResult<VfsNodeAttr> {
        Ok(VfsNodeAttr::new_dir(4096, 0))
    }

    fn parent(&self) -> Option<VfsNodeRef> {
        self.parent.lock().upgrade()
    }

    fn lookup(self: Arc<Self>, path: &str) -> VfsResult<VfsNodeRef> {
        let (name, rest) = split_path(path);
        let node = match name {
            "" | "." => Ok(self.clone() as VfsNodeRef),
            ".." => self.parent().ok_or(VfsError::NotFound),
            _ => self
                .children
                .lock()
                .get(name)
                .cloned()
                .ok_or(VfsError::NotFound),
        }?;

        if let Some(rest) = rest {
            node.lookup(rest)
        } else {
            Ok(node)
        }
    }

    fn read_dir(&self, start_idx: usize, dirents: &mut [VfsDirEntry]) -> VfsResult<usize> {
        let children = self.children.lock();
        let mut children = children.iter().skip(start_idx.max(2) - 2);
        for (i, ent) in dirents.iter_mut().enumerate() {
            match i + start_idx {
                0 => *ent = VfsDirEntry::new(".", VfsNodeType::Dir),
                1 => *ent = VfsDirEntry::new("..", VfsNodeType::Dir),
                _ => {
                    if let Some((name, node)) = children.next() {
                        *ent = VfsDirEntry::new(name, node.get_attr().unwrap().file_type());
                    } else {
                        return Ok(i);
                    }
                }
            }
        }
        Ok(dirents.len())
    }

    fn create(&self, path: &str, ty: VfsNodeType) -> VfsResult {
        log::debug!("create {:?} at ramfs: {}", ty, path);
        let (name, rest) = split_path(path);
        if let Some(rest) = rest {
            match name {
                "" | "." => self.create(rest, ty),
                ".." => self.parent().ok_or(VfsError::NotFound)?.create(rest, ty),
                _ => {
                    let subdir = self
                        .children
                        .lock()
                        .get(name)
                        .ok_or(VfsError::NotFound)?
                        .clone();
                    subdir.create(rest, ty)
                }
            }
        } else if name.is_empty() || name == "." || name == ".." {
            Ok(()) // already exists
        } else {
            self.create_node(name, ty)
        }
    }

    fn rename(&self, src_path: &str, dst_path: &str) -> VfsResult {
        let (src_parent, src_name) = self.rename_parent(src_path)?;
        let (dst_parent, dst_name) = self.rename_parent(dst_path)?;
        // This filesystem supports renaming within one directory, not moving.
        if !Arc::ptr_eq(&src_parent, &dst_parent) {
            return Err(VfsError::Unsupported);
        }
        let directory = src_parent
            .as_any()
            .downcast_ref::<DirNode>()
            .ok_or(VfsError::NotADirectory)?;
        let mut children = directory.children.lock();
        let source = children.get(src_name).ok_or(VfsError::NotFound)?;
        if src_name == dst_name {
            return Ok(());
        }
        if let Some(target) = children.get(dst_name) {
            let source_is_dir = source.get_attr()?.is_dir();
            let target_is_dir = target.get_attr()?.is_dir();
            if source_is_dir && !target_is_dir {
                return Err(VfsError::NotADirectory);
            }
            if !source_is_dir && target_is_dir {
                return Err(VfsError::IsADirectory);
            }
            if let Some(target_dir) = target.as_any().downcast_ref::<DirNode>() {
                if !target_dir.children.lock().is_empty() {
                    return Err(VfsError::DirectoryNotEmpty);
                }
            }
        }
        // Keep the same inode: open handles and file contents survive a rename.
        // Both directory updates happen while holding the single directory lock.
        let source = children.remove(src_name).ok_or(VfsError::NotFound)?;
        children.insert(dst_name.into(), source);
        Ok(())
    }

    fn remove(&self, path: &str) -> VfsResult {
        log::debug!("remove at ramfs: {}", path);
        let (name, rest) = split_path(path);
        if let Some(rest) = rest {
            match name {
                "" | "." => self.remove(rest),
                ".." => self.parent().ok_or(VfsError::NotFound)?.remove(rest),
                _ => {
                    let subdir = self
                        .children
                        .lock()
                        .get(name)
                        .ok_or(VfsError::NotFound)?
                        .clone();
                    subdir.remove(rest)
                }
            }
        } else if name.is_empty() || name == "." || name == ".." {
            Err(VfsError::InvalidInput) // remove '.' or '..
        } else {
            self.remove_node(name)
        }
    }

    axfs_vfs::impl_vfs_dir_default! {}
}

fn split_path(path: &str) -> (&str, Option<&str>) {
    let trimmed_path = path.trim_start_matches('/');
    trimmed_path.find('/').map_or((trimmed_path, None), |n| {
        (&trimmed_path[..n], Some(&trimmed_path[n + 1..]))
    })
}
