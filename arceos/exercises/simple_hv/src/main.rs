#![cfg_attr(feature = "axstd", no_std)]
#![cfg_attr(feature = "axstd", no_main)]
#![feature(asm_const)]
#![feature(riscv_ext_intrinsics)]

#[cfg(feature = "axstd")]
extern crate axstd as std;
extern crate alloc;
#[macro_use]
extern crate axlog;

mod task;
mod vcpu;
mod regs;
mod csrs;
mod sbi;
mod loader;
mod emulation;

use vcpu::VmCpuRegisters;
use riscv::register::{scause, sstatus, stval};
use csrs::defs::hstatus;
use tock_registers::LocalRegisterCopy;
use csrs::{RiscvCsrTrait, CSR};
use vcpu::_run_guest;
use sbi::SbiMessage;
use loader::load_vm_image;
use axhal::mem::PhysAddr;
use crate::regs::GprIndex::{A0, A1};

const VM_ENTRY: usize = 0x8020_0000;

#[cfg_attr(feature = "axstd", no_mangle)]
fn main() {
    ax_println!("Hypervisor ...");

    // A new address space for vm.
    let mut uspace = axmm::new_user_aspace().unwrap();

    // Load vm binary file into address space.
    if let Err(e) = load_vm_image("/sbin/skernel2", &mut uspace) {
        panic!("Cannot load app! {:?}", e);
    }

    // Setup context to prepare to enter guest mode.
    let mut ctx = VmCpuRegisters::default();
    prepare_guest_context(&mut ctx);

    // Setup pagetable for 2nd address mapping.
    let ept_root = uspace.page_table_root();
    prepare_vm_pgtable(ept_root);

    // Kick off vm and wait for it to exit.
    while !run_guest(&mut ctx, &uspace) {
    }

    ax_println!("Hypervisor ok!");
}

fn prepare_vm_pgtable(ept_root: PhysAddr) {
    let hgatp = 8usize << 60 | usize::from(ept_root) >> 12;
    unsafe {
        core::arch::asm!(
            "csrw hgatp, {hgatp}",
            hgatp = in(reg) hgatp,
        );
        core::arch::riscv64::hfence_gvma_all();
    }
}

fn run_guest(ctx: &mut VmCpuRegisters, uspace: &axmm::AddrSpace) -> bool {
    unsafe {
        _run_guest(ctx);
    }

    vmexit_handler(ctx, uspace)
}

fn vmexit_handler(ctx: &mut VmCpuRegisters, uspace: &axmm::AddrSpace) -> bool {
    use scause::{Exception, Trap};

    let scause = scause::read();
    match scause.cause() {
        Trap::Exception(Exception::VirtualSupervisorEnvCall) => {
            let sbi_msg = SbiMessage::from_regs(ctx.guest_regs.gprs.a_regs()).ok();
            ax_println!("VmExit Reason: VSuperEcall: {:?}", sbi_msg);
            if let Some(msg) = sbi_msg {
                match msg {
                    SbiMessage::Reset(_) => {
                        let a0 = ctx.guest_regs.gprs.reg(A0);
                        let a1 = ctx.guest_regs.gprs.reg(A1);
                        ax_println!("a0 = {:#x}, a1 = {:#x}", a0, a1);
                        assert_eq!(a0, 0x6688);
                        assert_eq!(a1, 0x1234);
                        ax_println!("Shutdown vm normally!");
                        return true;
                    },
                    _ => panic!("Unsupported SBI message: {:?}", msg),
                }
            } else {
                panic!("bad sbi message! ");
            }
        },
        Trap::Exception(Exception::IllegalInstruction) => {
            let instruction = read_guest_instruction(uspace, ctx.guest_regs.sepc);
            assert!(
                emulation::emulate_mhartid(instruction, &mut ctx.guest_regs.gprs),
                "Bad instruction: {:#x} sepc: {:#x}",
                instruction,
                ctx.guest_regs.sepc
            );
            // Only the successfully emulated 32-bit CSR read is consumed.
            ctx.guest_regs.sepc += 4;
        },
        Trap::Exception(Exception::LoadGuestPageFault) => {
            let instruction = read_guest_instruction(uspace, ctx.guest_regs.sepc);
            let fault_addr = (riscv::register::htval::read() << 2) | (stval::read() & 3);
            assert!(
                emulation::emulate_mmio_load(instruction, fault_addr, &mut ctx.guest_regs.gprs),
                "Unsupported guest load: instruction {:#x}, address {:#x}, sepc {:#x}",
                instruction,
                fault_addr,
                ctx.guest_regs.sepc
            );
            ctx.guest_regs.sepc += 4;
        },
        _ => {
            panic!(
                "Unhandled trap: {:?}, sepc: {:#x}, stval: {:#x}",
                scause.cause(),
                ctx.guest_regs.sepc,
                stval::read()
            );
        }
    }
    false
}

/// The exercise guest keeps VS translation disabled, so sepc is also a GPA.
/// Read through its owned address space instead of dereferencing a guest address.
fn read_guest_instruction(uspace: &axmm::AddrSpace, pc: usize) -> u32 {
    let mut bytes = [0u8; 4];
    uspace
        .read(pc.into(), &mut bytes)
        .expect("Guest instruction is outside mapped memory");
    u32::from_le_bytes(bytes)
}

fn prepare_guest_context(ctx: &mut VmCpuRegisters) {
    // Set hstatus
    let mut hstatus = LocalRegisterCopy::<usize, hstatus::Register>::new(
        riscv::register::hstatus::read().bits(),
    );
    // Set Guest bit in order to return to guest mode.
    hstatus.modify(hstatus::spv::Guest);
    // Set SPVP bit in order to accessing VS-mode memory from HS-mode.
    hstatus.modify(hstatus::spvp::Supervisor);
    CSR.hstatus.write_value(hstatus.get());
    ctx.guest_regs.hstatus = hstatus.get();

    // Set sstatus in guest mode.
    let mut sstatus = sstatus::read();
    sstatus.set_spp(sstatus::SPP::Supervisor);
    ctx.guest_regs.sstatus = sstatus.bits();
    // Return to entry to start vm.
    ctx.guest_regs.sepc = VM_ENTRY;
}
