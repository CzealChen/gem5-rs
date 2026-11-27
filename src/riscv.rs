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
    let inst = u64::from(instruction);
    let opcode = inst & 0x7f;
    let rd = ((inst >> 7) & 31) as usize;
    let funct3 = (inst >> 12) & 7;
    let rs1 = ((inst >> 15) & 31) as usize;
    let rs2 = ((inst >> 20) & 31) as usize;
    let funct7 = inst >> 25;
    let reg = |index| if index == 0 { 0 } else { registers[index] };
    let (lhs, rhs) = (reg(rs1), reg(rs2));
    let i_imm = sign_extend(inst >> 20, 12);
    let illegal = Fault {
        pc,
        reason: FaultReason::IllegalInstruction(instruction),
        address: None,
    };
    let mut commit = Commit {
        pc,
        instruction,
        next_pc: pc.wrapping_add(4),
        register_write: None,
    };
    let value = match opcode {
        0x37 => Some(sign_extend(inst & 0xffff_f000, 32)), // LUI
        0x17 => Some(pc.wrapping_add(sign_extend(inst & 0xffff_f000, 32))), // AUIPC
        0x6f => {
            let offset = (inst >> 31) << 20
                | (inst & 0x000f_f000)
                | ((inst >> 20) & 1) << 11
                | ((inst >> 21) & 0x3ff) << 1;
            let link = commit.next_pc;
            commit.next_pc = pc.wrapping_add(sign_extend(offset, 21));
            Some(link)
        }
        0x67 if funct3 == 0 => {
            let link = commit.next_pc;
            commit.next_pc = lhs.wrapping_add(i_imm) & !1;
            Some(link)
        }
        0x63 => {
            let taken = match funct3 {
                0 => lhs == rhs,
                1 => lhs != rhs,
                4 => (lhs as i64) < (rhs as i64),
                5 => (lhs as i64) >= (rhs as i64),
                6 => lhs < rhs,
                7 => lhs >= rhs,
                _ => return Err(illegal),
            };
            if taken {
                let offset = (inst >> 31) << 12
                    | ((inst >> 7) & 1) << 11
                    | ((inst >> 25) & 0x3f) << 5
                    | ((inst >> 8) & 0xf) << 1;
                commit.next_pc = pc.wrapping_add(sign_extend(offset, 13));
            }
            None
        }
        0x03 => {
            let (size, signed) = match funct3 {
                0..=3 => (1 << funct3, true),
                4..=6 => (1 << (funct3 - 4), false),
                _ => return Err(illegal),
            };
            let address = lhs.wrapping_add(i_imm);
            check_alignment(pc, address, size, AccessKind::Read)?;
            return Ok(Step::Memory(Pending {
                access: Access::Read { address, size },
                commit,
                load_rd: rd,
                signed,
            }));
        }
        0x23 => {
            let size = match funct3 {
                0..=3 => 1 << funct3,
                _ => return Err(illegal),
            };
            let offset = sign_extend((funct7 << 5) | rd as u64, 12);
            let address = lhs.wrapping_add(offset);
            check_alignment(pc, address, size, AccessKind::Write)?;
            return Ok(Step::Memory(Pending {
                access: Access::Write {
                    address,
                    data: rhs.to_le_bytes()[..size].to_vec(),
                },
                commit,
                load_rd: 0,
                signed: false,
            }));
        }
        0x13 => Some(match funct3 {
            0 => lhs.wrapping_add(i_imm),
            1 if inst >> 26 == 0 => lhs << (i_imm & 63),
            2 => u64::from((lhs as i64) < (i_imm as i64)),
            3 => u64::from(lhs < i_imm),
            4 => lhs ^ i_imm,
            5 if inst >> 26 == 0 => lhs >> (i_imm & 63),
            5 if inst >> 26 == 0x10 => ((lhs as i64) >> (i_imm & 63)) as u64,
            6 => lhs | i_imm,
            7 => lhs & i_imm,
            _ => return Err(illegal),
        }),
        0x33 => Some(match (funct3, funct7) {
            (0, 0) => lhs.wrapping_add(rhs),
            (0, 0x20) => lhs.wrapping_sub(rhs),
            (1, 0) => lhs << (rhs & 63),
            (2, 0) => u64::from((lhs as i64) < (rhs as i64)),
            (3, 0) => u64::from(lhs < rhs),
            (4, 0) => lhs ^ rhs,
            (5, 0) => lhs >> (rhs & 63),
            (5, 0x20) => ((lhs as i64) >> (rhs & 63)) as u64,
            (6, 0) => lhs | rhs,
            (7, 0) => lhs & rhs,
            _ => return Err(illegal),
        }),
        0x1b => {
            let lhs = lhs as u32;
            let result = match (funct3, funct7) {
                (0, _) => lhs.wrapping_add(i_imm as u32),
                (1, 0) => lhs << rs2,
                (5, 0) => lhs >> rs2,
                (5, 0x20) => ((lhs as i32) >> rs2) as u32,
                _ => return Err(illegal),
            };
            Some(sign_extend(u64::from(result), 32))
        }
        0x3b => {
            let (lhs, rhs) = (lhs as u32, rhs as u32);
            let result = match (funct3, funct7) {
                (0, 0) => lhs.wrapping_add(rhs),
                (0, 0x20) => lhs.wrapping_sub(rhs),
                (1, 0) => lhs << (rhs & 31),
                (5, 0) => lhs >> (rhs & 31),
                (5, 0x20) => ((lhs as i32) >> (rhs & 31)) as u32,
                _ => return Err(illegal),
            };
            Some(sign_extend(u64::from(result), 32))
        }
        0x0f if funct3 == 0 => None, // Prior transactions finish before execute.
        0x73 => {
            let reason = match instruction {
                0x0000_0073 => FaultReason::EnvironmentCall,
                0x0010_0073 => FaultReason::Breakpoint,
                _ => return Err(illegal),
            };
            return Err(Fault {
                pc,
                reason,
                address: None,
            });
        }
        _ => return Err(illegal),
    };
    check_alignment(pc, commit.next_pc, 4, AccessKind::Fetch)?;
    if rd != 0 {
        commit.register_write = value.map(|value| (rd, value));
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
        for instruction in [0x0ff0_000f, 0x8330_000f] {
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
