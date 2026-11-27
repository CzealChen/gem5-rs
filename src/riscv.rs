//! RV64I instruction semantics and transaction completions.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessKind {
    Fetch,
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultReason {
    IllegalInstruction(u32),
    EnvironmentCall,
    Breakpoint,
    Misaligned(AccessKind),
    AccessFault(AccessKind),
    ResponseMismatch(AccessKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub pc: u64,
    pub reason: FaultReason,
    pub address: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Access {
    Read { address: u64, size: usize },
    Write { address: u64, data: Vec<u8> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response<'a> {
    ReadData(&'a [u8]),
    WriteAck,
    Fault,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Commit {
    pub pc: u64,
    pub instruction: u32,
    pub next_pc: u64,
    pub register_write: Option<(usize, u64)>,
}

impl Commit {
    pub fn apply(&self, pc: &mut u64, registers: &mut [u64; 32]) {
        if let Some((rd, value)) = self.register_write {
            registers[rd] = value;
        }
        registers[0] = 0;
        *pc = self.next_pc;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Commit(Commit),
    Memory(Pending),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Pending {
    access: Access,
    commit: Commit,
    load_rd: usize,
    signed: bool,
}

impl Pending {
    pub fn access(&self) -> &Access {
        &self.access
    }

    pub fn complete(mut self, response: Response<'_>) -> Result<Commit, Fault> {
        let (kind, address) = match (&self.access, response) {
            (Access::Read { size, .. }, Response::ReadData(bytes)) if bytes.len() == *size => {
                let mut data = [0; 8];
                data[..*size].copy_from_slice(bytes);
                let value = u64::from_le_bytes(data);
                if self.load_rd != 0 {
                    let value = if self.signed {
                        sign_extend(value, size * 8)
                    } else {
                        value
                    };
                    self.commit.register_write = Some((self.load_rd, value));
                }
                return Ok(self.commit);
            }
            (Access::Write { .. }, Response::WriteAck) => return Ok(self.commit),
            (Access::Read { address, .. }, _) => (AccessKind::Read, *address),
            (Access::Write { address, .. }, _) => (AccessKind::Write, *address),
        };
        Err(response_fault(self.commit.pc, address, kind, response))
    }
}

pub fn fetch_address(pc: u64) -> Result<u64, Fault> {
    check_alignment(pc, pc, 4, AccessKind::Fetch)?;
    Ok(pc)
}

pub fn finish_fetch(pc: u64, response: Response<'_>) -> Result<u32, Fault> {
    fetch_address(pc)?;
    match response {
        Response::ReadData(bytes) => <[u8; 4]>::try_from(bytes)
            .map(u32::from_le_bytes)
            .map_err(|_| response_fault(pc, pc, AccessKind::Fetch, response)),
        _ => Err(response_fault(pc, pc, AccessKind::Fetch, response)),
    }
}

pub fn execute(pc: u64, instruction: u32, registers: &[u64; 32]) -> Result<Step, Fault> {
    fetch_address(pc)?;
    use riscv_isa::{Instruction::*, Target, Xlen, decode_full};

    let target = Target {
        xlen: Xlen::Rv64,
        ..Target::default()
    };
    let reg = |index: u32| {
        if index == 0 {
            0
        } else {
            registers[index as usize]
        }
    };
    let word = |value: u32| value as i32 as u64;
    let mut commit = Commit {
        pc,
        instruction,
        next_pc: pc.wrapping_add(4),
        register_write: None,
    };
    let load = |rd, rs1, offset: i32, size, signed, commit| {
        let address = reg(rs1).wrapping_add(offset as u64);
        check_alignment(pc, address, size, AccessKind::Read)?;
        Ok(Step::Memory(Pending {
            access: Access::Read { address, size },
            commit,
            load_rd: rd as usize,
            signed,
        }))
    };
    let store = |rs1, rs2, offset: i32, size, commit| {
        let address = reg(rs1).wrapping_add(offset as u64);
        check_alignment(pc, address, size, AccessKind::Write)?;
        Ok(Step::Memory(Pending {
            access: Access::Write {
                address,
                data: reg(rs2).to_le_bytes()[..size].to_vec(),
            },
            commit,
            load_rd: 0,
            signed: false,
        }))
    };
    let mut branch = |taken, offset: i32| {
        if taken {
            commit.next_pc = pc.wrapping_add(offset as u64);
        }
        None
    };
    let write = match decode_full(instruction, &target) {
        // The decoder supplies an unshifted U immediate and signed byte offsets.
        LUI { rd, imm } => Some((rd, word(imm << 12))),
        AUIPC { rd, imm } => Some((rd, pc.wrapping_add(word(imm << 12)))),
        JAL { rd, offset } => {
            let link = commit.next_pc;
            commit.next_pc = pc.wrapping_add(offset as u64);
            Some((rd, link))
        }
        JALR { rd, rs1, offset } => {
            let link = commit.next_pc;
            commit.next_pc = reg(rs1).wrapping_add(offset as u64) & !1;
            Some((rd, link))
        }
        BEQ { rs1, rs2, offset } => branch(reg(rs1) == reg(rs2), offset),
        BNE { rs1, rs2, offset } => branch(reg(rs1) != reg(rs2), offset),
        BLT { rs1, rs2, offset } => branch((reg(rs1) as i64) < reg(rs2) as i64, offset),
        BGE { rs1, rs2, offset } => branch(reg(rs1) as i64 >= reg(rs2) as i64, offset),
        BLTU { rs1, rs2, offset } => branch(reg(rs1) < reg(rs2), offset),
        BGEU { rs1, rs2, offset } => branch(reg(rs1) >= reg(rs2), offset),
        LB { rd, rs1, offset } => return load(rd, rs1, offset, 1, true, commit),
        LH { rd, rs1, offset } => return load(rd, rs1, offset, 2, true, commit),
        LW { rd, rs1, offset } => return load(rd, rs1, offset, 4, true, commit),
        LD { rd, rs1, offset } => return load(rd, rs1, offset, 8, true, commit),
        LBU { rd, rs1, offset } => return load(rd, rs1, offset, 1, false, commit),
        LHU { rd, rs1, offset } => return load(rd, rs1, offset, 2, false, commit),
        LWU { rd, rs1, offset } => return load(rd, rs1, offset, 4, false, commit),
        SB { rs1, rs2, offset } => return store(rs1, rs2, offset, 1, commit),
        SH { rs1, rs2, offset } => return store(rs1, rs2, offset, 2, commit),
        SW { rs1, rs2, offset } => return store(rs1, rs2, offset, 4, commit),
        SD { rs1, rs2, offset } => return store(rs1, rs2, offset, 8, commit),
        ADDI { rd, rs1, imm } => Some((rd, reg(rs1).wrapping_add(imm as u64))),
        SLTI { rd, rs1, imm } => Some((rd, u64::from((reg(rs1) as i64) < i64::from(imm)))),
        SLTIU { rd, rs1, imm } => Some((rd, u64::from(reg(rs1) < imm as u64))),
        XORI { rd, rs1, imm } => Some((rd, reg(rs1) ^ imm as u64)),
        ORI { rd, rs1, imm } => Some((rd, reg(rs1) | imm as u64)),
        ANDI { rd, rs1, imm } => Some((rd, reg(rs1) & imm as u64)),
        ADDIW { rd, rs1, imm } => Some((rd, word((reg(rs1) as u32).wrapping_add(imm as u32)))),
        SLLI { rd, rs1, shamt } => Some((rd, reg(rs1) << shamt)),
        SRLI { rd, rs1, shamt } => Some((rd, reg(rs1) >> shamt)),
        SRAI { rd, rs1, shamt } => Some((rd, ((reg(rs1) as i64) >> shamt) as u64)),
        SLLIW { rd, rs1, shamt } => Some((rd, word((reg(rs1) as u32) << shamt))),
        SRLIW { rd, rs1, shamt } => Some((rd, word((reg(rs1) as u32) >> shamt))),
        SRAIW { rd, rs1, shamt } => Some((rd, ((reg(rs1) as i32) >> shamt) as u64)),
        ADD { rd, rs1, rs2 } => Some((rd, reg(rs1).wrapping_add(reg(rs2)))),
        SUB { rd, rs1, rs2 } => Some((rd, reg(rs1).wrapping_sub(reg(rs2)))),
        SLL { rd, rs1, rs2 } => Some((rd, reg(rs1) << (reg(rs2) & 63))),
        SLT { rd, rs1, rs2 } => Some((rd, u64::from((reg(rs1) as i64) < reg(rs2) as i64))),
        SLTU { rd, rs1, rs2 } => Some((rd, u64::from(reg(rs1) < reg(rs2)))),
        XOR { rd, rs1, rs2 } => Some((rd, reg(rs1) ^ reg(rs2))),
        SRL { rd, rs1, rs2 } => Some((rd, reg(rs1) >> (reg(rs2) & 63))),
        SRA { rd, rs1, rs2 } => Some((rd, ((reg(rs1) as i64) >> (reg(rs2) & 63)) as u64)),
        OR { rd, rs1, rs2 } => Some((rd, reg(rs1) | reg(rs2))),
        AND { rd, rs1, rs2 } => Some((rd, reg(rs1) & reg(rs2))),
        ADDW { rd, rs1, rs2 } => Some((rd, word((reg(rs1) as u32).wrapping_add(reg(rs2) as u32)))),
        SUBW { rd, rs1, rs2 } => Some((rd, word((reg(rs1) as u32).wrapping_sub(reg(rs2) as u32)))),
        SLLW { rd, rs1, rs2 } => Some((rd, word((reg(rs1) as u32) << (reg(rs2) & 31)))),
        SRLW { rd, rs1, rs2 } => Some((rd, word((reg(rs1) as u32) >> (reg(rs2) & 31)))),
        SRAW { rd, rs1, rs2 } => Some((rd, ((reg(rs1) as i32) >> (reg(rs2) & 31)) as u64)),
        FENCE { .. } => None,
        // 0.3.1 decodes fm=0. RV64I treats the other FENCE modes (including TSO)
        // as normal fences; all prior transactions have completed here.
        UNIMP if instruction & 0x707f == 0x000f => None,
        other => {
            let reason = match other {
                ECALL => FaultReason::EnvironmentCall,
                EBREAK => FaultReason::Breakpoint,
                _ => FaultReason::IllegalInstruction(instruction),
            };
            return Err(Fault {
                pc,
                reason,
                address: None,
            });
        }
    };
    check_alignment(pc, commit.next_pc, 4, AccessKind::Fetch)?;
    if let Some((rd, value)) = write
        && rd != 0
    {
        commit.register_write = Some((rd as usize, value));
    }
    Ok(Step::Commit(commit))
}

fn sign_extend(value: u64, bits: usize) -> u64 {
    let shift = 64 - bits;
    (((value << shift) as i64) >> shift) as u64
}

fn check_alignment(pc: u64, address: u64, size: usize, kind: AccessKind) -> Result<(), Fault> {
    if address & (size as u64 - 1) == 0 {
        Ok(())
    } else {
        Err(Fault {
            pc,
            reason: FaultReason::Misaligned(kind),
            address: Some(address),
        })
    }
}

fn response_fault(pc: u64, address: u64, kind: AccessKind, response: Response<'_>) -> Fault {
    let reason = match response {
        Response::Fault => FaultReason::AccessFault(kind),
        _ => FaultReason::ResponseMismatch(kind),
    };
    Fault {
        pc,
        reason,
        address: Some(address),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn immediate(opcode: u32, funct3: u32, imm: i32) -> u32 {
        ((imm as u32 & 0xfff) << 20) | (1 << 15) | (funct3 << 12) | (3 << 7) | opcode
    }

    fn register(opcode: u32, funct3: u32, funct7: u32) -> u32 {
        (funct7 << 25) | (2 << 20) | (1 << 15) | (funct3 << 12) | (3 << 7) | opcode
    }

    fn commit(pc: u64, instruction: u32, regs: &[u64; 32]) -> Commit {
        let Step::Commit(commit) = execute(pc, instruction, regs).unwrap() else {
            panic!("expected an immediate commit: {instruction:08x}");
        };
        commit
    }

    fn pending(pc: u64, instruction: u32, regs: &[u64; 32]) -> Pending {
        let Step::Memory(pending) = execute(pc, instruction, regs).unwrap() else {
            panic!("expected a memory intent: {instruction:08x}");
        };
        pending
    }

    #[test]
    fn integer_immediates_and_x0() {
        let mut regs = [0; 32];
        regs[0] = 123;
        regs[1] = u64::MAX;
        let cases = [
            (immediate(0x13, 0, 2), 1), // ADDI wraps
            (immediate(0x13, 1, 63), 0x8000_0000_0000_0000),
            (immediate(0x13, 2, 1), 1),
            (immediate(0x13, 3, -2), 0),
            (immediate(0x13, 4, -2), 1),
            (immediate(0x13, 5, 63), 1),
            (immediate(0x13, 5, 0x43f), u64::MAX),
            (immediate(0x13, 6, 0), u64::MAX),
            (
                immediate(0x13, 7, 0xabc_i32 - 0x1000),
                0xffff_ffff_ffff_fabc,
            ),
            (0x8000_01b7, 0xffff_ffff_8000_0000), // LUI
            (0xffff_f197, 0x200),                 // AUIPC x3, -4096 at 0x1200
            (0x0070_0193, 7),                     // ADDI x3, x0, 7
        ];
        for (instruction, value) in cases {
            let mut pc = 0x1200;
            let next = commit(pc, instruction, &regs);
            assert_eq!(next.instruction, instruction);
            assert_eq!(next.pc, 0x1200);
            assert_eq!(next.register_write, Some((3, value)), "{instruction:08x}");
            let mut committed_regs = regs;
            next.apply(&mut pc, &mut committed_regs);
            assert_eq!(
                (pc, committed_regs[3], committed_regs[0]),
                (0x1204, value, 0)
            );
        }
        assert_eq!(commit(0, 0x0010_0013, &regs).register_write, None);
        assert_eq!(commit(u64::MAX - 3, 0x0000_0013, &regs).next_pc, 0);
    }

    #[test]
    fn integer_register_operations_use_xlen() {
        let mut regs = [0; 32];
        regs[1] = 0x8000_0000_0000_0001;
        regs[2] = 66;
        let cases = [
            (0, 0, 0x8000_0000_0000_0043),
            (0, 0x20, 0x7fff_ffff_ffff_ffbf),
            (1, 0, 4),
            (2, 0, 1),
            (3, 0, 0),
            (4, 0, 0x8000_0000_0000_0043),
            (5, 0, 0x2000_0000_0000_0000),
            (5, 0x20, 0xe000_0000_0000_0000),
            (6, 0, 0x8000_0000_0000_0043),
            (7, 0, 0),
        ];
        for (funct3, funct7, value) in cases {
            let instruction = register(0x33, funct3, funct7);
            assert_eq!(
                commit(0, instruction, &regs).register_write,
                Some((3, value)),
                "{instruction:08x}"
            );
        }
    }

    #[test]
    fn word_operations_truncate_mask_and_sign_extend() {
        let mut regs = [0; 32];
        regs[1] = 0xdead_beef_8000_0001;
        regs[2] = 32;
        let cases = [
            (immediate(0x1b, 0, -1), 0xffff_ffff_8000_0000),
            (immediate(0x1b, 1, 31), 0xffff_ffff_8000_0000),
            (immediate(0x1b, 5, 31), 1),
            (immediate(0x1b, 5, 0x41f), u64::MAX),
            (register(0x3b, 0, 0), 0xffff_ffff_8000_0021),
            (register(0x3b, 0, 0x20), 0x7fff_ffe1),
            (register(0x3b, 1, 0), 0xffff_ffff_8000_0001),
            (register(0x3b, 5, 0), 0xffff_ffff_8000_0001),
            (register(0x3b, 5, 0x20), 0xffff_ffff_8000_0001),
        ];
        for (instruction, value) in cases {
            assert_eq!(
                commit(0, instruction, &regs).register_write,
                Some((3, value)),
                "{instruction:08x}"
            );
        }
    }

    #[test]
    fn branches_compare_and_only_check_taken_targets() {
        let mut regs = [0; 32];
        regs[1] = u64::MAX;
        regs[2] = 1;
        for (funct3, next) in [
            (0, 0x1004),
            (1, 0xff8),
            (4, 0xff8),
            (5, 0x1004),
            (6, 0x1004),
            (7, 0xff8),
        ] {
            // B<cond> x1,x2,-8
            let instruction = 0xfe20_8ce3 | (funct3 << 12);
            let commit = commit(0x1000, instruction, &regs);
            assert_eq!(
                (commit.next_pc, commit.register_write),
                (next, None),
                "{instruction:08x}"
            );
        }
        // BEQ x1,x2,+2 is initially not taken, then taken after a register change.
        assert_eq!(commit(0x1000, 0x0020_8163, &regs).next_pc, 0x1004);
        regs[2] = u64::MAX;
        assert_eq!(
            execute(0x1000, 0x0020_8163, &regs),
            Err(Fault {
                pc: 0x1000,
                reason: FaultReason::Misaligned(AccessKind::Fetch),
                address: Some(0x1002),
            })
        );
    }

    #[test]
    fn jumps_compute_target_and_link_before_register_write() {
        let mut regs = [0; 32];
        let jal = commit(0x1000, 0xff9f_f1ef, &regs); // JAL x3,-8
        assert_eq!(
            (jal.next_pc, jal.register_write),
            (0xff8, Some((3, 0x1004)))
        );
        regs[1] = 0x200d;
        let jalr = commit(0x1000, 0xffc0_80e7, &regs); // JALR x1,-4(x1)
        assert_eq!(
            (jalr.next_pc, jalr.register_write),
            (0x2008, Some((1, 0x1004)))
        );
        regs[1] = 0x200f;
        assert_eq!(
            execute(0x1000, 0xffc0_80e7, &regs),
            Err(Fault {
                pc: 0x1000,
                reason: FaultReason::Misaligned(AccessKind::Fetch),
                address: Some(0x200a),
            })
        );
    }

    #[test]
    fn loads_complete_with_width_and_extension_from_little_endian_data() {
        let mut regs = [0; 32];
        regs[1] = 0x1010;
        let bytes = [0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88];
        let cases = [
            (0, 1, 0xffff_ffff_ffff_ff81),
            (1, 2, 0xffff_ffff_ffff_8281),
            (2, 4, 0xffff_ffff_8483_8281),
            (3, 8, 0x8887_8685_8483_8281),
            (4, 1, 0x81),
            (5, 2, 0x8281),
            (6, 4, 0x8483_8281),
        ];
        for (funct3, size, value) in cases {
            let wait = pending(0x800, immediate(0x03, funct3, -8), &regs);
            assert_eq!(
                wait.access(),
                &Access::Read {
                    address: 0x1008,
                    size
                }
            );
            let completed = wait.complete(Response::ReadData(&bytes[..size])).unwrap();
            assert_eq!(
                (completed.next_pc, completed.register_write),
                (0x804, Some((3, value)))
            );
        }
        regs[1] = u64::MAX;
        let wait = pending(0x800, 0x0010_c003, &regs); // LBU x0,1(x1)
        assert_eq!(
            wait.access(),
            &Access::Read {
                address: 0,
                size: 1
            }
        );
        assert_eq!(
            wait.complete(Response::ReadData(&[0xff]))
                .unwrap()
                .register_write,
            None
        );
    }

    #[test]
    fn stores_issue_owned_low_bytes_and_acknowledge_before_commit() {
        let mut regs = [0; 32];
        regs[1] = 0x1010;
        regs[2] = 0x8877_6655_4433_2211;
        let expected = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        for (funct3, size) in [(0, 1), (1, 2), (2, 4), (3, 8)] {
            let instruction = 0xfe20_8c23 | (funct3 << 12); // S<width> x2,-8(x1)
            let wait = pending(0x800, instruction, &regs);
            assert_eq!(
                wait.access(),
                &Access::Write {
                    address: 0x1008,
                    data: expected[..size].to_vec()
                }
            );
            assert_eq!(
                wait.complete(Response::WriteAck).unwrap(),
                Commit {
                    pc: 0x800,
                    instruction,
                    next_pc: 0x804,
                    register_write: None,
                }
            );
        }
    }

    #[test]
    fn memory_faults_name_origin_pc_operation_and_address() {
        let mut regs = [0; 32];
        regs[1] = 0x1001;
        assert_eq!(
            execute(0x800, 0x0000_a183, &regs),
            Err(Fault {
                pc: 0x800,
                reason: FaultReason::Misaligned(AccessKind::Read),
                address: Some(0x1001),
            })
        );
        assert_eq!(
            execute(0x800, 0x0020_b023, &regs),
            Err(Fault {
                pc: 0x800,
                reason: FaultReason::Misaligned(AccessKind::Write),
                address: Some(0x1001),
            })
        );
        regs[1] = 0x1000;
        assert_eq!(
            pending(0x800, 0x0000_a003, &regs).complete(Response::Fault),
            Err(Fault {
                pc: 0x800,
                reason: FaultReason::AccessFault(AccessKind::Read),
                address: Some(0x1000),
            })
        );
        assert_eq!(
            pending(0x800, 0x0020_b023, &regs).complete(Response::Fault),
            Err(Fault {
                pc: 0x800,
                reason: FaultReason::AccessFault(AccessKind::Write),
                address: Some(0x1000),
            })
        );
        assert_eq!(
            pending(0x800, 0x0000_a183, &regs).complete(Response::ReadData(&[0; 2])),
            Err(Fault {
                pc: 0x800,
                reason: FaultReason::ResponseMismatch(AccessKind::Read),
                address: Some(0x1000),
            })
        );
        assert_eq!(
            pending(0x800, 0x0020_b023, &regs).complete(Response::ReadData(&[0; 8])),
            Err(Fault {
                pc: 0x800,
                reason: FaultReason::ResponseMismatch(AccessKind::Write),
                address: Some(0x1000),
            })
        );
    }

    #[test]
    fn fetch_checks_alignment_response_shape_and_byte_order() {
        assert_eq!(fetch_address(0x1080), Ok(0x1080));
        assert_eq!(
            fetch_address(0x1082),
            Err(Fault {
                pc: 0x1082,
                reason: FaultReason::Misaligned(AccessKind::Fetch),
                address: Some(0x1082),
            })
        );
        assert_eq!(
            finish_fetch(0x1080, Response::ReadData(&[0x93, 0x01, 0x70, 0])),
            Ok(0x0070_0193)
        );
        assert_eq!(
            finish_fetch(0x1080, Response::Fault),
            Err(Fault {
                pc: 0x1080,
                reason: FaultReason::AccessFault(AccessKind::Fetch),
                address: Some(0x1080),
            })
        );
        assert_eq!(
            finish_fetch(0x1080, Response::WriteAck),
            Err(Fault {
                pc: 0x1080,
                reason: FaultReason::ResponseMismatch(AccessKind::Fetch),
                address: Some(0x1080),
            })
        );
    }

    #[test]
    fn fences_produce_sequential_commits_and_system_instructions_report_the_cause() {
        let regs = [0; 32];
        for instruction in [0x0ff0_000f, 0x8330_000f, 0xf420_808f] {
            let fence = commit(0x100, instruction, &regs);
            assert_eq!((fence.next_pc, fence.register_write), (0x104, None));
        }
        for (instruction, reason) in [
            (0x0000_0073, FaultReason::EnvironmentCall),
            (0x0010_0073, FaultReason::Breakpoint),
        ] {
            assert_eq!(
                execute(0x100, instruction, &regs),
                Err(Fault {
                    pc: 0x100,
                    reason,
                    address: None
                })
            );
        }
        for instruction in [
            0,
            0x0000_0001,
            0x0220_81b3,
            0x0200_919b,
            0x0000_100f,
            0x0010_1173,
            0x3020_0073, // MRET and WFI decode even with privileged=false in 0.3.1.
            0x1050_0073,
            0x0400_9193, // SLLI with reserved imm[6].
            0x8330_100f, // A nonzero fm does not turn FENCE.I into FENCE.
        ] {
            assert_eq!(
                execute(0x100, instruction, &regs),
                Err(Fault {
                    pc: 0x100,
                    reason: FaultReason::IllegalInstruction(instruction),
                    address: None,
                })
            );
        }
    }
}
