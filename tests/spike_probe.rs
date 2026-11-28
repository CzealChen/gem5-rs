//! Opt-in real Spike capability test; default Cargo builds do not link C++.
use gem5rs::riscv::{self, Access, Response, Step};
use std::process::Command;

#[test]
#[ignore = "run devenv shell -- just check-spike-probe"]
fn real_spike_steps_and_resumes_after_full_state_sync() {
    let probe = std::env::var_os("GEM5RS_SPIKE_PROBE").expect("probe executable required");
    let run = || {
        let output = Command::new(&probe).output().expect("start Spike probe");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    let output = run();
    assert_eq!(output, run(), "fresh initial state must reproduce results");
    let mut lines = output.lines();
    let mut registers = [0; 32];
    let mut pc = 0x8000_0000;
    registers[1] = pc;
    let mut ram = [0; 8];
    for (index, instruction) in [0x00700513, 0x00550593, 0x10b0b023, 0x1000b603, 0x00c506b3]
        .into_iter()
        .enumerate()
    {
        if index == 4 {
            for (index, value) in registers.iter_mut().enumerate() {
                *value = index as u64 * 3;
            }
            registers[0] = 0;
        }
        let commit = match riscv::execute(pc, instruction, &registers).unwrap() {
            Step::Commit(commit) => commit,
            Step::Memory(pending) => match pending.access() {
                Access::Write { address, data } => {
                    assert_eq!(*address, 0x8000_0100);
                    ram.copy_from_slice(data);
                    pending.complete(Response::WriteAck).unwrap()
                }
                Access::Read { address, size } => {
                    assert_eq!((*address, *size), (0x8000_0100, 8));
                    pending.complete(Response::ReadData(&ram)).unwrap()
                }
            },
        };
        commit.apply(&mut pc, &mut registers);
        let actual: Vec<u64> = lines
            .next()
            .unwrap()
            .split_whitespace()
            .map(|value| value.parse().unwrap())
            .collect();
        let expected: Vec<u64> = std::iter::once(pc).chain(registers).collect();
        assert_eq!(actual, expected, "first difference at instruction {index}");
    }
    assert_eq!(lines.next(), Some("breakpoint 3 2147483668"));
    assert_eq!(lines.next(), None);
}
