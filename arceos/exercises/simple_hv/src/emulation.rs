use crate::regs::{GeneralPurposeRegisters, GprIndex};

/// Values exposed by this exercise's virtual CPU and read-only MMIO register.
const VIRTUAL_HART_ID: usize = 0x1234;
const MMIO_REGISTER_ADDR: usize = 64;
const MMIO_REGISTER_VALUE: usize = 0x6688;

/// Emulate `csrr rd, mhartid` (CSRRS with rs1=x0).
pub fn emulate_mhartid(instruction: u32, regs: &mut GeneralPurposeRegisters) -> bool {
    if instruction & !0xf80 != 0xf140_2073 {
        return false;
    }
    let rd = GprIndex::from_raw((instruction >> 7) & 0x1f).unwrap();
    regs.set_reg(rd, VIRTUAL_HART_ID);
    true
}

/// Emulate an RV64 LD from the virtual device's single 64-bit register.
pub fn emulate_mmio_load(
    instruction: u32,
    fault_addr: usize,
    regs: &mut GeneralPurposeRegisters,
) -> bool {
    // LD is a 32-bit instruction with opcode 0b0000011 and funct3 0b011.
    if instruction & 0x707f != 0x3003 || fault_addr != MMIO_REGISTER_ADDR {
        return false;
    }
    let rs1 = GprIndex::from_raw((instruction >> 15) & 0x1f).unwrap();
    let offset = (instruction as i32 >> 20) as isize;
    if regs.reg(rs1).wrapping_add(offset as usize) != fault_addr {
        return false;
    }
    let rd = GprIndex::from_raw((instruction >> 7) & 0x1f).unwrap();
    regs.set_reg(rd, MMIO_REGISTER_VALUE);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csr_read_uses_decoded_destination_and_preserves_zero() {
        let mut regs = GeneralPurposeRegisters::default();
        assert!(emulate_mhartid(0xf140_22f3, &mut regs)); // csrr t0, mhartid
        assert_eq!(regs.reg(GprIndex::T0), VIRTUAL_HART_ID);
        assert!(emulate_mhartid(0xf140_2073, &mut regs)); // csrr zero, mhartid
        assert_eq!(regs.reg(GprIndex::Zero), 0);
        assert!(!emulate_mhartid(0xf140_12f3, &mut regs)); // csrrw writes the CSR
        assert!(!emulate_mhartid(0xf110_22f3, &mut regs)); // a different CSR
    }

    #[test]
    fn device_read_checks_access_and_sign_extends_offset() {
        let mut regs = GeneralPurposeRegisters::default();
        assert!(emulate_mmio_load(0x0400_3503, 64, &mut regs)); // ld a0,64(zero)
        assert_eq!(regs.reg(GprIndex::A0), MMIO_REGISTER_VALUE);
        regs.set_reg(GprIndex::T0, 72);
        assert!(emulate_mmio_load(0xff82_b303, 64, &mut regs)); // ld t1,-8(t0)
        assert_eq!(regs.reg(GprIndex::T1), MMIO_REGISTER_VALUE);
        regs.set_reg(GprIndex::A0, 7);
        assert!(!emulate_mmio_load(0x0400_3503, 72, &mut regs)); // other device
        assert!(!emulate_mmio_load(0x0480_3503, 64, &mut regs)); // mismatched VA
        assert!(!emulate_mmio_load(0x0400_2503, 64, &mut regs)); // LW, not LD
        assert_eq!(regs.reg(GprIndex::A0), 7);
    }
}
