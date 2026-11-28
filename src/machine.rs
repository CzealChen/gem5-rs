//! Opt-in M-only trap execution (RV64I + Zicsr, optional machine timer).
//! Semantics: RISC-V privileged specification, Machine-Level ISA, sections
//! Machine Status/Trap Setup/Trap Handling and Trap-Return Instructions.
//! https://docs.riscv.org/reference/isa/priv/machine.html
use crate::riscv::{AccessKind, Commit, Fault, FaultReason};

const MIE: u64 = 1 << 3;
const MPIE: u64 = 1 << 7;
const MPP: u64 = 3 << 11;

/// Architectural M-mode state checked independently by reference execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub privilege: u64,
    pub mstatus: u64,
    pub mie: u64,
    pub mip: u64,
    pub mtvec: u64,
    pub mscratch: u64,
    pub mepc: u64,
    pub mcause: u64,
    pub mtval: u64,
}

#[derive(Debug)]
pub struct Machine {
    mstatus: u64,
    mie: u64,
    timer_pending: bool,
    mtvec: u64,
    mscratch: u64,
    mepc: u64,
    mcause: u64,
    mtval: u64,
}

impl Default for Machine {
    fn default() -> Self {
        Self {
            mstatus: MPP,
            mie: 0,
            timer_pending: false,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
        }
    }
}

impl Machine {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            privilege: 3,
            mstatus: self.mstatus,
            mie: self.mie,
            mip: u64::from(self.timer_pending) << 7,
            mtvec: self.mtvec,
            mscratch: self.mscratch,
            mepc: self.mepc,
            mcause: self.mcause,
            mtval: self.mtval,
        }
    }

    fn read(&self, csr: u32) -> Option<u64> {
        Some(match csr {
            0x300 => self.mstatus,
            0x301 => (2 << 62) | (1 << 8), // RV64I; only M privilege exists.
            0x304 => self.mie,
            0x344 => u64::from(self.timer_pending) << 7,
            0x305 => self.mtvec,
            0x340 => self.mscratch,
            0x341 => self.mepc,
            0x342 => self.mcause,
            0x343 => self.mtval,
            0xf14 => 0, // single hart
            _ => return None,
        })
    }

    fn write(&mut self, csr: u32, value: u64) -> bool {
        match csr {
            0x300 => self.mstatus = (value & (MIE | MPIE)) | MPP,
            0x301 => {} // WARL fixed ISA
            0x304 => self.mie = value & (1 << 7),
            0x344 => {}                       // MTIP is a read-only hardware level.
            0x305 => self.mtvec = value & !3, // direct mode only
            0x340 => self.mscratch = value,
            0x341 => self.mepc = value & !3, // IALIGN=32
            0x342 => self.mcause = value,
            0x343 => self.mtval = value,
            _ => return false,
        }
        true
    }

    /// CSR/mret only; other instructions go to the RV64I executor.
    pub fn execute(
        &mut self,
        pc: u64,
        instruction: u32,
        registers: &[u64; 32],
    ) -> Option<Result<Commit, Fault>> {
        let mut commit = Commit {
            pc,
            instruction,
            next_pc: pc.wrapping_add(4),
            register_write: None,
        };
        if instruction == 0x3020_0073 {
            commit.next_pc = self.mepc;
            self.mstatus = MPP | MPIE | ((self.mstatus & MPIE) >> 4);
            return Some(Ok(commit));
        }
        let op = (instruction >> 12) & 7;
        if instruction & 0x7f != 0x73 || !matches!(op, 1 | 2 | 3 | 5 | 6 | 7) {
            return None;
        }
        let illegal = || {
            Err(Fault {
                pc,
                reason: FaultReason::IllegalInstruction(instruction),
                address: None,
            })
        };
        let csr = instruction >> 20;
        let Some(old) = self.read(csr) else {
            return Some(illegal());
        };
        let rs1 = ((instruction >> 15) & 31) as usize;
        let rd = ((instruction >> 7) & 31) as usize;
        let operand = if op & 4 != 0 {
            rs1 as u64
        } else if rs1 == 0 {
            0
        } else {
            registers[rs1]
        };
        let writes = op & 3 == 1 || rs1 != 0;
        if writes {
            let value = match op & 3 {
                1 => operand,
                2 => old | operand,
                _ => old & !operand,
            };
            if !self.write(csr, value) {
                return Some(illegal());
            }
        }
        if rd != 0 {
            commit.register_write = Some((rd, old));
        }
        Some(Ok(commit))
    }

    /// Interrupts retain the next instruction PC; pending is cleared only by
    /// the timer device, not by entry or software writes to mip.
    pub fn timer_interrupt(&mut self, pc: u64, pending: bool) -> Option<u64> {
        self.timer_pending = pending;
        if pending && self.mie & (1 << 7) != 0 && self.mstatus & MIE != 0 {
            self.mepc = pc;
            self.mcause = (1 << 63) | 7;
            self.mtval = 0;
            self.mstatus = MPP | ((self.mstatus & MIE) << 4);
            Some(self.mtvec)
        } else {
            None
        }
    }

    /// Trap entry does not retire the faulting instruction.
    pub fn enter(&mut self, fault: &Fault) -> Option<u64> {
        let (cause, value) = match fault.reason {
            FaultReason::Misaligned(AccessKind::Fetch) => (0, fault.address.unwrap_or(fault.pc)),
            FaultReason::AccessFault(AccessKind::Fetch) => (1, fault.address.unwrap_or(fault.pc)),
            FaultReason::IllegalInstruction(word) => (2, u64::from(word)),
            FaultReason::Breakpoint => (3, fault.pc),
            FaultReason::Misaligned(AccessKind::Read) => (4, fault.address.unwrap_or(0)),
            FaultReason::AccessFault(AccessKind::Read) => (5, fault.address.unwrap_or(0)),
            FaultReason::Misaligned(AccessKind::Write) => (6, fault.address.unwrap_or(0)),
            FaultReason::AccessFault(AccessKind::Write) => (7, fault.address.unwrap_or(0)),
            FaultReason::EnvironmentCall => (11, 0),
            FaultReason::ResponseMismatch(_) => return None, // simulator protocol error
        };
        self.mepc = fault.pc & !3;
        self.mcause = cause;
        self.mtval = value;
        self.mstatus = MPP | ((self.mstatus & MIE) << 4);
        Some(self.mtvec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csr(
        machine: &mut Machine,
        address: u32,
        op: u32,
        rd: u32,
        rs1: u32,
        value: u64,
    ) -> Result<Commit, Fault> {
        let mut regs = [0; 32];
        regs[rs1 as usize] = value;
        machine
            .execute(
                0x100,
                address << 20 | rs1 << 15 | op << 12 | rd << 7 | 0x73,
                &regs,
            )
            .unwrap()
    }

    #[test]
    fn csr_forms_zero_operands_readonly_and_warl() {
        let mut m = Machine::default();
        csr(&mut m, 0x340, 1, 0, 1, 0x88).unwrap();
        assert_eq!(
            csr(&mut m, 0x340, 2, 2, 0, 999).unwrap().register_write,
            Some((2, 0x88))
        );
        csr(&mut m, 0x340, 6, 0, 1, 0).unwrap();
        assert_eq!(m.mscratch, 0x89);
        csr(&mut m, 0x340, 7, 0, 8, 0).unwrap();
        assert_eq!(m.mscratch, 0x81);
        csr(&mut m, 0x340, 3, 0, 1, 0x80).unwrap();
        assert_eq!(m.mscratch, 1);
        csr(&mut m, 0x340, 5, 0, 0, 0).unwrap();
        assert_eq!(m.mscratch, 0);
        assert!(csr(&mut m, 0xf14, 2, 1, 0, 0).is_ok());
        assert!(csr(&mut m, 0xf14, 2, 0, 1, 0).is_err());
        assert!(csr(&mut m, 0xf14, 1, 0, 0, 0).is_err());
        assert!(csr(&mut m, 0xfff, 2, 1, 0, 0).is_err());
        csr(&mut m, 0x305, 1, 0, 1, 0x123).unwrap();
        csr(&mut m, 0x341, 1, 0, 1, 0x987).unwrap();
        assert_eq!((m.mtvec, m.mepc), (0x120, 0x984));
    }

    #[test]
    fn trap_and_return_preserve_interrupt_stack_and_fault_metadata() {
        let mut m = Machine::default();
        m.write(0x305, 0x8000_1000);
        m.write(0x300, MIE);
        let fault = Fault {
            pc: 0x8000_2000,
            reason: FaultReason::EnvironmentCall,
            address: None,
        };
        assert_eq!(m.enter(&fault), Some(0x8000_1000));
        assert_eq!(
            (m.mepc, m.mcause, m.mtval, m.mstatus),
            (fault.pc, 11, 0, MPP | MPIE)
        );
        m.write(0x341, fault.pc + 4);
        let ret = m.execute(0, 0x3020_0073, &[0; 32]).unwrap().unwrap();
        assert_eq!((ret.next_pc, m.mstatus), (fault.pc + 4, MPP | MPIE | MIE));
        m.enter(&Fault {
            reason: FaultReason::IllegalInstruction(0xffff_ffff),
            ..fault
        });
        assert_eq!((m.mcause, m.mtval), (2, 0xffff_ffff));
        m.enter(&Fault {
            reason: FaultReason::Misaligned(AccessKind::Write),
            address: Some(17),
            ..fault
        });
        assert_eq!((m.mcause, m.mtval, m.mstatus), (6, 17, MPP));
    }

    #[test]
    fn timer_is_level_triggered_maskable_and_does_not_advance_pc() {
        let mut m = Machine::default();
        m.write(0x305, 0x1000);
        assert_eq!(m.timer_interrupt(0x400, true), None);
        assert_eq!(m.read(0x344), Some(128));
        m.write(0x300, MIE);
        assert_eq!(m.timer_interrupt(0x400, true), None); // MTIE is off
        m.write(0x304, 128);
        assert_eq!(m.timer_interrupt(0x400, true), Some(0x1000));
        assert_eq!((m.mepc, m.mcause, m.mtval), (0x400, (1 << 63) | 7, 0));
        assert_eq!(m.timer_interrupt(0x1000, true), None); // trap masks MIE
        m.write(0x344, 0);
        assert_eq!(m.read(0x344), Some(128)); // software cannot acknowledge MTIP
        m.execute(0, 0x3020_0073, &[0; 32]).unwrap().unwrap();
        assert_eq!(m.timer_interrupt(0x400, true), Some(0x1000)); // still pending
        m.execute(0, 0x3020_0073, &[0; 32]).unwrap().unwrap();
        assert_eq!(m.timer_interrupt(0x400, false), None); // device acknowledged
        assert_eq!(m.read(0x344), Some(0));
    }
}
