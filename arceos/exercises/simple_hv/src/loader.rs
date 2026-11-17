use std::fs::File;
use std::io::{self, Read};

use axhal::mem::PAGE_SIZE_4K;
use axhal::paging::MappingFlags;
use axmm::AddrSpace;

use crate::VM_ENTRY;

pub fn load_vm_image(fname: &str, uspace: &mut AddrSpace) -> io::Result<()> {
    ax_println!("app: {}", fname);
    let mut file = File::open(fname)?;
    let mut buf = alloc::vec::Vec::new();
    file.read_to_end(&mut buf)?;
    assert!(
        !buf.is_empty() && buf.len() <= PAGE_SIZE_4K,
        "Guest image must fit in one nonempty page"
    );

    uspace
        .map_alloc(
            VM_ENTRY.into(),
            PAGE_SIZE_4K,
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE | MappingFlags::USER,
            true,
        )
        .unwrap();

    let (paddr, _, _) = uspace.page_table().query(VM_ENTRY.into()).unwrap();
    ax_println!("paddr: {:#x}", paddr);
    // Copy only the bytes actually read; the old 64-byte buffer was overread by
    // copying PAGE_SIZE_4K bytes from it.
    uspace.write(VM_ENTRY.into(), &buf).unwrap();
    Ok(())
}
