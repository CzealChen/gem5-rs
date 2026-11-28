//! gem5 RV64 instruction ABI definitions, independent of CPU and runtime.
//!
//! Source: gem5 9d368896ce119c17e1289dcbb18f33624724b843,
//! include/gem5/asm/generic/m5ops.h and util/m5/src/abi/riscv/m5op.S.
//! Defining the ABI does not enable it in the default RV64I decoder.

/// RV64 arguments occupy a0/a1; every supported instruction writes a0.
pub const ARG0: usize = 10;
pub const ARG1: usize = 11;
pub const RESULT: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Operation {
    Rpns = 0x07,
    Exit = 0x21,
    Fail = 0x22,
    WorkBegin = 0x5a,
    WorkEnd = 0x5b,
}

impl Operation {
    pub const ALL: [Self; 5] = [
        Self::Rpns,
        Self::Exit,
        Self::Fail,
        Self::WorkBegin,
        Self::WorkEnd,
    ];

    pub const fn instruction(self) -> u32 {
        0x7b | ((self as u32) << 25)
    }

    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Rpns => "m5_rpns",
            Self::Exit => "m5_exit",
            Self::Fail => "m5_fail",
            Self::WorkBegin => "m5_work_begin",
            Self::WorkEnd => "m5_work_end",
        }
    }
}

/// Emit stubs from the same operation table used by the future shared decoder.
/// This is a build artifact; guest sources must not maintain duplicate opcodes.
pub fn guest_assembly() -> String {
    use std::fmt::Write;
    let mut output = String::from(".section .text\n.balign 4\n.option norvc\n");
    for op in Operation::ALL {
        let name = op.symbol();
        writeln!(
            output,
            ".global {name}\n.type {name}, @function\n{name}:\n.long {:#010x}\nret\n.size {name}, .-{name}",
            op.instruction()
        )
        .expect("writing to String is infallible");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings_match_fixed_gem5_libm5_and_remain_disabled_in_rv64i() {
        // Independent literal words from the pinned libm5 encoding formula.
        let words = [0x0e00007b, 0x4200007b, 0x4400007b, 0xb400007b, 0xb600007b];
        for (operation, word) in Operation::ALL.into_iter().zip(words) {
            assert_eq!(operation.instruction(), word);
            let fault = crate::riscv::execute(0, word, &[0; 32]).unwrap_err();
            assert_eq!(
                fault.reason,
                crate::riscv::FaultReason::IllegalInstruction(word)
            );
        }
        assert_eq!((ARG0, ARG1, RESULT), (10, 11, 10));
    }
}
