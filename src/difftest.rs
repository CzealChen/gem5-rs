//! Streaming architectural comparison against an isolated, versioned reference.
use crate::{
    elf,
    observation::{MemoryEffect, Observer, Retirement},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const SPIKE_COMMIT: &str = "d140cbc0ae4ef4fc64ad323f3f175ff15f6dca56";
const MAX_MESSAGE: usize = 16384;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub adapter: String,
    pub timeout_ms: u64,
    pub expected_exit_code: u64,
    pub expected_failure: bool,
}

impl Config {
    pub fn validate(self) -> Result<Self, String> {
        if self.adapter.is_empty() {
            return Err("reference adapter path must not be empty".into());
        }
        if self.timeout_ms == 0 || self.timeout_ms > 60_000 {
            return Err("reference timeout must be in 1..=60000 ms".into());
        }
        Ok(self)
    }
}

/// One worker performs blocking pipe I/O; the simulation thread imposes a
/// deadline on the whole exchange, including a potentially blocked write.
pub struct Reference {
    child: Child,
    requests: Option<SyncSender<String>>,
    responses: Receiver<Result<String, String>>,
    worker: Option<JoinHandle<()>>,
    timeout: Duration,
}

impl Reference {
    pub fn start(config: &Config) -> Result<Self, String> {
        config.clone().validate()?;
        let mut child = Command::new(&config.adapter)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("start reference: {e}"))?;
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let (send, receive) = mpsc::sync_channel::<String>(1);
        let (replies, responses) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            while let Ok(request) = receive.recv() {
                let reply = (|| {
                    input
                        .write_all(request.as_bytes())
                        .map_err(|e| format!("reference write: {e}"))?;
                    input
                        .write_all(b"\n")
                        .and_then(|()| input.flush())
                        .map_err(|e| format!("reference flush: {e}"))?;
                    let mut bytes = Vec::new();
                    (&mut output)
                        .take((MAX_MESSAGE + 1) as u64)
                        .read_until(b'\n', &mut bytes)
                        .map_err(|e| format!("reference read: {e}"))?;
                    if bytes.is_empty() {
                        return Err("reference EOF before response".into());
                    }
                    if bytes.len() > MAX_MESSAGE || bytes.last() != Some(&b'\n') {
                        return Err("reference oversized or unterminated response".into());
                    }
                    String::from_utf8(bytes)
                        .map(|line| line.trim_end().to_owned())
                        .map_err(|_| "reference non-UTF8 response".into())
                })();
                let failed = reply.is_err();
                if replies.send(reply).is_err() || failed {
                    break;
                }
            }
        });
        let mut reference = Self {
            child,
            requests: Some(send),
            responses,
            worker: Some(worker),
            timeout: Duration::from_millis(config.timeout_ms),
        };
        let hello = reference.exchange("hello".into())?;
        if hello != format!("hello 1 {SPIKE_COMMIT} rv64i pc_gpr_mem") {
            return Err(format!("reference handshake mismatch: {hello}"));
        }
        Ok(reference)
    }

    pub fn exchange(&mut self, request: String) -> Result<String, String> {
        if request.len() >= MAX_MESSAGE || request.contains(['\n', '\r']) {
            return Err("invalid reference request framing".into());
        }
        self.requests
            .as_ref()
            .ok_or("reference already closed")?
            .send(request)
            .map_err(|_| "reference worker stopped".to_string())?;
        let reply = self
            .responses
            .recv_timeout(self.timeout)
            .map_err(|e| format!("reference response timeout/disconnect: {e}"))
            .and_then(|reply| reply);
        if reply.is_err() {
            let _ = self.child.kill();
        }
        let reply = reply?;
        if reply.starts_with("error ") {
            return Err(format!("reference rejected request: {reply}"));
        }
        Ok(reply)
    }

    fn ok(&mut self, request: String) -> Result<(), String> {
        let reply = self.exchange(request)?;
        if reply != "ok" {
            return Err(format!("reference expected ok, received {reply}"));
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), String> {
        let reply = self.exchange("quit".into())?;
        if reply != "bye" {
            return Err(format!("reference expected bye, received {reply}"));
        }
        self.requests.take();
        let start = Instant::now();
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|e| format!("reference wait: {e}"))?
            {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("reference abnormal exit: {status}"))
                };
            }
            if start.elapsed() >= self.timeout {
                return Err("reference shutdown timeout".into());
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Drop for Reference {
    fn drop(&mut self) {
        self.requests.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Statistics {
    pub protocol_version: u32,
    pub reference_commit: String,
    pub compared: u64,
    pub synced_m5: u64,
    pub synced_console: u64,
    pub synced_exit: u64,
    pub passed: bool,
}

#[derive(Default)]
pub struct Platform {
    pub console: Vec<u64>,
    pub exit: Vec<(u64, usize)>,
}

pub struct Driver {
    reference: Reference,
    config: Config,
    platform: Platform,
    reference_pc: u64,
    reference_registers: [u64; 32],
    stats: Statistics,
    recent: VecDeque<String>,
    failure: Option<String>,
}

fn state_command(command: &str, pc: u64, registers: &[u64; 32]) -> String {
    let mut text = format!("{command} {pc}");
    for reg in registers {
        text.push_str(&format!(" {reg}"));
    }
    text
}

impl Driver {
    pub fn start(
        config: Config,
        ram: std::ops::Range<u64>,
        image: &elf::Image,
        platform: Platform,
    ) -> Result<Self, String> {
        if ram.end <= ram.start {
            return Err("reference RAM range must be nonempty".into());
        }
        let mut reference = Reference::start(&config)?;
        let command = format!("init rv64i {} {}", ram.start, ram.end - ram.start);
        reference.ok(state_command(
            &command,
            image.entry,
            &image.initial_registers(),
        ))?;
        for segment in &image.segments {
            for (index, bytes) in segment.data.chunks(4096).enumerate() {
                use std::fmt::Write as _;
                let mut command = format!("load {} ", segment.address + (index * 4096) as u64);
                for byte in bytes {
                    write!(command, "{byte:02x}").unwrap();
                }
                reference.ok(command)?;
            }
        }
        Ok(Self {
            reference,
            config,
            platform,
            reference_pc: image.entry,
            reference_registers: image.initial_registers(),
            stats: Statistics {
                protocol_version: 1,
                reference_commit: SPIKE_COMMIT.into(),
                ..Statistics::default()
            },
            recent: VecDeque::new(),
            failure: None,
        })
    }

    pub fn finish(&mut self, exit: Option<crate::experiment::Exit>) -> Result<Statistics, String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let expected = crate::experiment::Exit {
            code: self.config.expected_exit_code,
            failed: self.config.expected_failure,
        };
        if exit != Some(expected) {
            return Err(format!("difftest expected exit {expected:?}, got {exit:?}"));
        }
        self.reference.finish()?;
        self.stats.passed = true;
        Ok(self.stats.clone())
    }

    fn compare(&mut self, commit: &Retirement) -> Result<(), String> {
        if commit.pc != self.reference_pc {
            return Err(format!(
                "pre-PC DUT={:#x}, reference={:#x}",
                commit.pc, self.reference_pc
            ));
        }
        let mut special = false;
        if let Some(service) = commit.service {
            if crate::m5::Operation::decode(commit.instruction) != Some(service)
                || commit.memory.is_some()
            {
                return Err("invalid completed service observation".into());
            }
            self.stats.synced_m5 += 1;
            special = true;
        } else if let Some(memory) = &commit.memory
            && memory.write
            && commit.instruction & 0x7f == 0x23
        {
            if memory.data.len() == 1 && self.platform.console.contains(&memory.address) {
                self.stats.synced_console += 1;
                special = true;
            } else if self
                .platform
                .exit
                .contains(&(memory.address, memory.data.len()))
            {
                self.stats.synced_exit += 1;
                special = true;
            }
        }
        if special {
            let instruction = self.reference.exchange("inspect".into())?;
            if instruction != format!("instruction {} {}", commit.pc, commit.instruction) {
                return Err(format!("platform instruction mismatch: {instruction}"));
            }
            if commit.next_pc != commit.pc.wrapping_add(4) {
                return Err("platform operation has nonsequential next PC".into());
            }
            if let Some(memory) = &commit.memory {
                // Recognize the MMIO store using its real decoding and the
                // previously compared state, not only a DUT-reported address.
                let decoded =
                    crate::riscv::execute(commit.pc, commit.instruction, &self.reference_registers)
                        .map_err(|fault| format!("platform store decode: {fault:?}"))?;
                let crate::riscv::Step::Memory(pending) = decoded else {
                    return Err("platform access is not a decoded store".into());
                };
                if pending.access()
                    != &(crate::riscv::Access::Write {
                        address: memory.address,
                        data: memory.data.clone(),
                    })
                {
                    return Err(
                        "platform store address/data disagree with decoded instruction".into(),
                    );
                }
            }
            self.reference
                .ok(state_command("sync", commit.next_pc, &commit.registers))?;
        } else {
            let reply = self.reference.exchange("step".into())?;
            if reply.starts_with("fault ") {
                return Err(format!("reference architectural {reply}"));
            }
            let state = parse_state(&reply)?;
            if (state.pc, state.instruction) != (commit.pc, commit.instruction) {
                return Err(format!(
                    "instruction DUT={:#x}/{:#x}, reference={:#x}/{:#x}",
                    commit.pc, commit.instruction, state.pc, state.instruction
                ));
            }
            if state.next_pc != commit.next_pc {
                return Err(format!(
                    "next-PC DUT={:#x}, reference={:#x}",
                    commit.next_pc, state.next_pc
                ));
            }
            for (index, (&dut, &reference)) in
                commit.registers.iter().zip(&state.registers).enumerate()
            {
                if dut != reference {
                    return Err(format!("x{index} DUT={dut:#x}, reference={reference:#x}"));
                }
            }
            if state.memory != commit.memory {
                return Err(format!(
                    "memory DUT={:?}, reference={:?}",
                    commit.memory, state.memory
                ));
            }
            self.stats.compared += 1;
        }
        self.reference_pc = commit.next_pc;
        self.reference_registers = commit.registers;
        Ok(())
    }
}

impl Observer for Driver {
    fn faulted(&mut self, fault: &crate::riscv::Fault) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        use crate::riscv::{AccessKind, FaultReason};
        let cause = match fault.reason {
            FaultReason::Misaligned(AccessKind::Fetch) => 0,
            FaultReason::AccessFault(AccessKind::Fetch) => 1,
            FaultReason::IllegalInstruction(_) => 2,
            FaultReason::Breakpoint => 3,
            FaultReason::Misaligned(AccessKind::Read) => 4,
            FaultReason::AccessFault(AccessKind::Read) => 5,
            FaultReason::Misaligned(AccessKind::Write) => 6,
            FaultReason::AccessFault(AccessKind::Write) => 7,
            FaultReason::EnvironmentCall => 11,
            FaultReason::ResponseMismatch(_) => {
                return Err("DUT memory protocol error is not an architectural trap".into());
            }
        };
        let result = (|| {
            if fault.pc != self.reference_pc {
                return Err("fault pre-PC differs".to_string());
            }
            let response = self.reference.exchange("step".into())?;
            let fields: Vec<_> = response.split_whitespace().collect();
            if fields.len() != 6 || fields[0] != "fault" {
                return Err(format!("DUT fault {fault:?}, reference {response}"));
            }
            let values = fields[1..]
                .iter()
                .map(|v| {
                    v.parse::<u64>()
                        .map_err(|_| "invalid reference fault field".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if values[0] != fault.pc
                || values[2] != cause
                || values[3] != fault.pc
                || fault.address.is_some_and(|address| values[4] != address)
                || matches!(fault.reason, FaultReason::IllegalInstruction(word) if u64::from(word) != values[1])
            {
                return Err(format!("DUT fault {fault:?}, reference {response}"));
            }
            Ok(())
        })();
        if let Err(error) = result {
            let error = format!(
                "difftest first fault difference at pc {:#x}: {error}",
                fault.pc
            );
            self.failure = Some(error.clone());
            return Err(error);
        }
        self.failure = Some(format!(
            "DUT architectural fault matched reference: {fault:?}"
        ));
        Ok(())
    }

    fn retired(&mut self, retirement: &Retirement) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if let Err(error) = self.compare(retirement) {
            let error = format!(
                "difftest first difference at pc {:#x}: {error}; recent: {:?}",
                retirement.pc, self.recent
            );
            self.failure = Some(error.clone());
            return Err(error);
        }
        if self.recent.len() == 16 {
            self.recent.pop_front();
        }
        self.recent.push_back(format!(
            "{:#x}:{:08x}",
            retirement.pc, retirement.instruction
        ));
        Ok(())
    }
}

fn parse_state(reply: &str) -> Result<Retirement, String> {
    let mut fields = reply.split_whitespace();
    if fields.next() != Some("state") {
        return Err(format!("reference expected state, received {reply}"));
    }
    fn number(fields: &mut std::str::SplitWhitespace<'_>) -> Result<u64, String> {
        fields
            .next()
            .ok_or("reference missing state field")?
            .parse()
            .map_err(|_| "reference invalid integer".into())
    }
    let pc = number(&mut fields)?;
    let instruction =
        u32::try_from(number(&mut fields)?).map_err(|_| "reference instruction exceeds u32")?;
    let next_pc = number(&mut fields)?;
    let mut registers = [0; 32];
    for reg in &mut registers {
        *reg = number(&mut fields)?;
    }
    if registers[0] != 0 {
        return Err("reference violated x0".into());
    }
    let memory = match number(&mut fields)? {
        0 => None,
        1 => {
            let write = match fields.next() {
                Some("read") => false,
                Some("write") => true,
                _ => return Err("reference invalid memory kind".into()),
            };
            let address = number(&mut fields)?;
            let size = number(&mut fields)?;
            if !matches!(size, 1 | 2 | 4 | 8) {
                return Err("reference invalid memory width".into());
            }
            let value = number(&mut fields)?;
            Some(MemoryEffect {
                address,
                write,
                data: value.to_le_bytes()[..size as usize].to_vec(),
            })
        }
        _ => return Err("reference multiple memory effects outside RV64I scope".into()),
    };
    if fields.next().is_some() {
        return Err("reference extra state fields".into());
    }
    Ok(Retirement {
        pc,
        instruction,
        next_pc,
        registers,
        memory,
        service: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_parser_rejects_partial_extra_and_invalid_state() {
        let valid = format!("state 4096 19 4100{} 0", " 0".repeat(32));
        assert_eq!(parse_state(&valid).unwrap().next_pc, 4100);
        for invalid in [
            "state 4096".to_string(),
            valid.replace(" 19 ", " 4294967296 "),
            format!("{valid} extra"),
            format!("state 4096 19 4100 1{} 0", " 0".repeat(31)),
            format!("state 4096 19 4100{} 1 read 8192 3 0", " 0".repeat(32)),
        ] {
            assert!(parse_state(&invalid).is_err(), "{invalid}");
        }
        let sources: serde_json::Value =
            serde_json::from_str(include_str!("../workloads/am/sources.json")).unwrap();
        assert_eq!(sources["spike"]["commit"].as_str(), Some(SPIKE_COMMIT));
    }
}
