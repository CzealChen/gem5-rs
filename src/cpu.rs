//! In-order RV64I request source and architectural commit trace.

use crate::memory::{Operation, ResponseResult, Source, SourceAction, SourceEvent};
use crate::riscv::{self, Access, Pending, Step};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct RegisterWrite {
    pub register: u64,
    pub value: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Commit {
    pub at: u64,
    pub pc: u64,
    pub instruction: u32,
    pub writeback: Option<RegisterWrite>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub reason: String,
    pub pc: u64,
    pub addr: Option<u64>,
    architectural: Option<riscv::Fault>,
}

enum Phase {
    Start,
    Fetch(u64),
    Execute(u32),
    Data(u64, Pending),
    Stopped,
}

pub struct Cpu {
    pc: u64,
    machine: Option<crate::machine::Machine>,
    timer: Option<crate::timer::Line>,
    timer_interrupts: u64,
    registers: [u64; 32],
    cycle: u64,
    phase: Phase,
    next_id: u64,
    commits: Vec<Commit>,
    committed: u64,
    retain_trace: bool,
    fault: Option<Fault>,
    experiment: Option<crate::experiment::Experiment>,
    instruction_limit: bool,
    observer: Option<Box<dyn crate::observation::Observer>>,
    observation_error: Option<String>,
}

impl Cpu {
    pub fn new(entry: u64, stack_pointer: u64, cycle: u64) -> Self {
        let mut registers = [0; 32];
        registers[2] = stack_pointer;
        Self {
            pc: entry,
            machine: None,
            timer: None,
            timer_interrupts: 0,
            registers,
            cycle,
            phase: Phase::Start,
            next_id: 0,
            commits: Vec::new(),
            committed: 0,
            retain_trace: true,
            fault: None,
            experiment: None,
            instruction_limit: false,
            observer: None,
            observation_error: None,
        }
    }

    pub fn with_machine_mode(mut self, enabled: bool) -> Self {
        self.machine = enabled.then(crate::machine::Machine::default);
        self
    }

    pub fn with_timer(mut self, timer: crate::timer::Line) -> Self {
        self.timer = Some(timer);
        self
    }

    pub fn timer_interrupts(&self) -> u64 {
        self.timer_interrupts
    }

    pub fn pc(&self) -> u64 {
        self.pc
    }

    pub fn with_experiment(mut self, experiment: crate::experiment::Experiment) -> Self {
        self.experiment = Some(experiment);
        self
    }

    pub fn experiment(&self) -> Option<&crate::experiment::Experiment> {
        self.experiment.as_ref()
    }

    pub fn instruction_limit(&self) -> bool {
        self.instruction_limit
    }

    pub fn with_observer(mut self, observer: impl crate::observation::Observer + 'static) -> Self {
        self.observer = Some(Box::new(observer));
        self
    }

    pub fn observation_error(&self) -> Option<&str> {
        self.observation_error.as_deref()
    }

    pub fn registers(&self) -> &[u64; 32] {
        &self.registers
    }

    pub fn commits(&self) -> &[Commit] {
        &self.commits
    }

    pub fn with_trace(mut self, retain: bool) -> Self {
        self.retain_trace = retain;
        self
    }
    pub fn committed(&self) -> u64 {
        self.committed
    }

    pub fn fault(&self) -> Option<&Fault> {
        self.fault.as_ref()
    }

    fn send(&mut self, at: u64, address: u64, operation: Operation) -> SourceAction {
        let id = self.next_id;
        // One request is outstanding at a time, so IDs can wrap.
        self.next_id = id.wrapping_add(1);
        SourceAction::Send {
            at,
            id,
            address,
            operation,
        }
    }

    fn fetch(&mut self, at: u64) -> Result<SourceAction, Fault> {
        let address = riscv::fetch_address(self.pc).map_err(|fault| isa_fault(fault, None))?;
        self.phase = Phase::Fetch(self.next_id);
        Ok(self.send(at, address, Operation::Fetch { size: 4 }))
    }

    fn execute(&mut self, at: u64, instruction: u32) -> Result<SourceAction, Fault> {
        if self
            .experiment
            .as_ref()
            .is_some_and(|experiment| self.committed >= experiment.budget())
        {
            self.instruction_limit = true;
            return Ok(SourceAction::Stop("instruction budget exhausted".into()));
        }
        if let Some(machine) = &mut self.machine {
            let pending = self
                .timer
                .as_ref()
                .is_some_and(|timer| timer.borrow().pending(at));
            if let Some(observer) = &mut self.observer
                && let Err(error) = observer.timer_pending(pending)
            {
                self.observation_error = Some(error.clone());
                return Err(Fault {
                    reason: error,
                    pc: self.pc,
                    addr: None,
                    architectural: None,
                });
            }
            if let Some(vector) = machine.timer_interrupt(self.pc, pending) {
                self.observe_trap(vector)?;
                self.timer_interrupts += 1;
                self.pc = vector;
                return self.fetch(at);
            }
        }
        if let Some(experiment) = &mut self.experiment
            && let Some(op) = crate::m5::Operation::decode(instruction)
        {
            let value = experiment
                .execute(
                    op,
                    [
                        self.registers[crate::m5::ARG0],
                        self.registers[crate::m5::ARG1],
                    ],
                    at,
                    self.committed,
                )
                .map_err(|reason| Fault {
                    reason,
                    pc: self.pc,
                    addr: None,
                    architectural: None,
                })?;
            let exit = experiment.exit();
            let commit = riscv::Commit {
                pc: self.pc,
                instruction,
                next_pc: self.pc.wrapping_add(4),
                register_write: Some((crate::m5::RESULT, value)),
            };
            self.retire(at, commit, None, Some(op))?;
            if let Some(exit) = exit {
                self.phase = Phase::Stopped;
                return Ok(SourceAction::Exit(exit.code));
            }
            return self.fetch(at);
        }
        if let Some(machine) = &mut self.machine
            && let Some(result) = machine.execute(self.pc, instruction, &self.registers)
        {
            return self.commit(at, result.map_err(|fault| isa_fault(fault, None))?, None);
        }
        match riscv::execute(self.pc, instruction, &self.registers)
            .map_err(|fault| isa_fault(fault, None))?
        {
            Step::Commit(commit) => self.commit(at, commit, None),
            Step::Memory(pending) => {
                let (address, operation) = match pending.access() {
                    Access::Read { address, size } => (*address, Operation::Read { size: *size }),
                    Access::Write { address, data } => {
                        (*address, Operation::Write { data: data.clone() })
                    }
                };
                self.phase = Phase::Data(self.next_id, pending);
                Ok(self.send(at, address, operation))
            }
        }
    }

    fn commit(
        &mut self,
        at: u64,
        commit: riscv::Commit,
        memory: Option<crate::observation::MemoryEffect>,
    ) -> Result<SourceAction, Fault> {
        self.retire(at, commit, memory, None)?;
        self.fetch(at)
    }

    fn observe_trap(&mut self, vector: u64) -> Result<(), Fault> {
        if let Some(observer) = &mut self.observer {
            let trap = crate::observation::Trap {
                pc: self.pc,
                next_pc: vector,
                registers: self.registers,
                machine: self
                    .machine
                    .as_ref()
                    .expect("trap requires machine mode")
                    .snapshot(),
            };
            if let Err(error) = observer.trapped(&trap) {
                self.observation_error = Some(error.clone());
                return Err(Fault {
                    reason: error,
                    pc: self.pc,
                    addr: None,
                    architectural: None,
                });
            }
        }
        Ok(())
    }

    fn retire(
        &mut self,
        at: u64,
        commit: riscv::Commit,
        memory: Option<crate::observation::MemoryEffect>,
        service: Option<crate::m5::Operation>,
    ) -> Result<(), Fault> {
        self.committed = self.committed.checked_add(1).ok_or_else(|| Fault {
            reason: "commit count overflow".into(),
            pc: commit.pc,
            addr: None,
            architectural: None,
        })?;
        commit.apply(&mut self.pc, &mut self.registers);
        if self.retain_trace {
            self.commits.push(Commit {
                at,
                pc: commit.pc,
                instruction: commit.instruction,
                writeback: commit
                    .register_write
                    .map(|(register, value)| RegisterWrite {
                        register: register as u64,
                        value,
                    }),
            });
        }
        if let Some(observer) = &mut self.observer {
            let retirement = crate::observation::Retirement {
                pc: commit.pc,
                instruction: commit.instruction,
                next_pc: self.pc,
                registers: self.registers,
                memory,
                service,
                machine: self.machine.as_ref().map(crate::machine::Machine::snapshot),
            };
            if let Err(error) = observer.retired(&retirement) {
                self.observation_error = Some(error.clone());
                return Err(Fault {
                    reason: error,
                    pc: commit.pc,
                    addr: None,
                    architectural: None,
                });
            }
        }
        Ok(())
    }
}

impl Source for Cpu {
    fn on_event(&mut self, at: u64, event: SourceEvent) -> Vec<SourceAction> {
        let phase = std::mem::replace(&mut self.phase, Phase::Stopped);
        let action = match (phase, event) {
            (Phase::Start, SourceEvent::Wake) => self.fetch(at),
            (Phase::Execute(instruction), SourceEvent::Wake) => self.execute(at, instruction),
            (Phase::Fetch(id), SourceEvent::Response(response)) if id == response.id => {
                let (response, target_fault) = isa_response(&response.result);
                riscv::finish_fetch(self.pc, response)
                    .map_err(|fault| isa_fault(fault, target_fault))
                    .map(|instruction| {
                        let Some(next_at) = at.checked_add(self.cycle) else {
                            return SourceAction::Stop(format!(
                                "execution tick overflow: {at} + {}",
                                self.cycle
                            ));
                        };
                        self.phase = Phase::Execute(instruction);
                        SourceAction::WakeAt(next_at)
                    })
            }
            (Phase::Data(id, pending), SourceEvent::Response(response)) if id == response.id => {
                let memory = if self.observer.is_some() {
                    match (pending.access(), &response.result) {
                        (Access::Read { address, .. }, ResponseResult::ReadData(data)) => {
                            Some(crate::observation::MemoryEffect {
                                address: *address,
                                write: false,
                                data: data.clone(),
                            })
                        }
                        (Access::Write { address, data }, ResponseResult::WriteAck) => {
                            Some(crate::observation::MemoryEffect {
                                address: *address,
                                write: true,
                                data: data.clone(),
                            })
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                let (response, target_fault) = isa_response(&response.result);
                pending
                    .complete(response)
                    .map_err(|fault| isa_fault(fault, target_fault))
                    .and_then(|commit| self.commit(at, commit, memory))
            }
            (phase, _) => {
                self.phase = phase;
                return Vec::new();
            }
        };
        vec![match action {
            Ok(action) => action,
            Err(fault) => {
                if let (Some(machine), Some(architectural)) =
                    (&mut self.machine, &fault.architectural)
                    && let Some(vector) = machine.enter(architectural)
                {
                    if let Err(error) = self.observe_trap(vector) {
                        let reason = error.reason.clone();
                        self.fault = Some(error);
                        return vec![SourceAction::Stop(reason)];
                    }
                    self.pc = vector;
                    return vec![self.fetch(at).expect("aligned direct trap vector")];
                }
                if let (Some(observer), Some(architectural)) =
                    (&mut self.observer, &fault.architectural)
                    && let Err(error) = observer.faulted(architectural)
                {
                    self.observation_error = Some(error);
                }
                let reason = fault.reason.clone();
                self.fault = Some(fault);
                SourceAction::Stop(reason)
            }
        }]
    }
}

fn isa_response(response: &ResponseResult) -> (riscv::Response<'_>, Option<crate::memory::Fault>) {
    match response {
        ResponseResult::ReadData(data) => (riscv::Response::ReadData(data), None),
        ResponseResult::WriteAck => (riscv::Response::WriteAck, None),
        ResponseResult::Fault(fault) => (riscv::Response::Fault, Some(*fault)),
    }
}

fn isa_fault(fault: riscv::Fault, target: Option<crate::memory::Fault>) -> Fault {
    let reason = match target {
        Some(target) => format!("{:?}: {target:?}", fault.reason),
        None => format!("{:?}", fault.reason),
    };
    Fault {
        reason,
        pc: fault.pc,
        addr: fault.address,
        architectural: Some(fault),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{Machine, Route, RunLimits, RunResult, StopReason, Target};

    const LD: u32 = 0x0001_3183; // ld x3, 0(x2)
    const ADDI: u32 = 0x0011_8013; // addi x0, x3, 1
    const SD: u32 = 0x0430_3023; // sd x3, 64(x0)

    fn program(cpu: &mut Cpu, words: &[u32], latency: u64, limits: RunLimits) -> RunResult {
        let mut machine = Machine::new(vec![0]).unwrap();
        machine
            .add_route(Route {
                base: 0,
                target: Target::FixedRam {
                    size_bytes: 64,
                    latency,
                    capacity: 1,
                },
            })
            .unwrap();
        machine
            .add_route(Route {
                base: 64,
                target: Target::ExitMmio {
                    width: 8,
                    latency: 4,
                },
            })
            .unwrap();
        let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        machine.load_ram(0, &bytes).unwrap();
        machine.load_ram(32, &42u64.to_le_bytes()).unwrap();
        machine.add_source(0, 0, cpu).unwrap();
        machine.run(limits).unwrap()
    }

    const LIMITS: RunLimits = RunLimits {
        events: 100,
        ticks: u64::MAX,
    };
    const PROGRAM: &[u32] = &[LD, ADDI, SD];

    #[test]
    fn fetch_execute_data_and_exit_ack_timeline() {
        let mut cpu = Cpu::new(0, 32, 2);
        let result = program(&mut cpu, PROGRAM, 3, LIMITS);
        assert_eq!((result.stop, result.tick), (StopReason::Exit(42), 22));
        let transfers: Vec<_> = result
            .transactions
            .iter()
            .map(|tx| {
                (
                    tx.request.id,
                    tx.request.address,
                    &tx.request.operation,
                    tx.arrived_at,
                    tx.completed_at.unwrap(),
                )
            })
            .collect();
        assert_eq!(
            transfers,
            [
                (0, 0, &Operation::Fetch { size: 4 }, 0, 3),
                (1, 32, &Operation::Read { size: 8 }, 5, 8),
                (2, 4, &Operation::Fetch { size: 4 }, 8, 11),
                (3, 8, &Operation::Fetch { size: 4 }, 13, 16),
                (
                    4,
                    64,
                    &Operation::Write {
                        data: vec![42, 0, 0, 0, 0, 0, 0, 0],
                    },
                    18,
                    22,
                ),
            ]
        );
        assert_eq!(
            cpu.commits(),
            [
                Commit {
                    at: 8,
                    pc: 0,
                    instruction: LD,
                    writeback: Some(RegisterWrite {
                        register: 3,
                        value: 42,
                    }),
                },
                Commit {
                    at: 13,
                    pc: 4,
                    instruction: ADDI,
                    writeback: None,
                },
                Commit {
                    at: 22,
                    pc: 8,
                    instruction: SD,
                    writeback: None,
                },
            ]
        );
        assert_eq!(cpu.pc(), 12);
        assert_eq!(cpu.registers()[..4], [0, 0, 32, 42]);
    }

    #[test]
    fn repeated_runs_reproduce_the_trace_and_ram_latency_moves_commits() {
        let mut first = Cpu::new(0, 32, 2);
        let mut repeated = Cpu::new(0, 32, 2);
        let mut slower = Cpu::new(0, 32, 2);
        let result = program(&mut first, PROGRAM, 3, LIMITS);
        assert_eq!(program(&mut repeated, PROGRAM, 3, LIMITS), result);
        assert_eq!(repeated.commits(), first.commits());

        let slower_result = program(&mut slower, PROGRAM, 6, LIMITS);
        assert_eq!(
            (slower_result.stop, slower_result.tick),
            (StopReason::Exit(42), 34)
        );
        for ((baseline, slow), expected_tick) in first
            .commits()
            .iter()
            .zip(slower.commits())
            .zip([14, 22, 34])
        {
            assert_eq!(
                *slow,
                Commit {
                    at: expected_tick,
                    ..*baseline
                }
            );
        }
        assert_eq!(slower.commits().len(), 3);
    }

    #[test]
    fn budgets_preserve_the_last_committed_state() {
        let mut before_load_response = Cpu::new(0, 32, 2);
        let result = program(
            &mut before_load_response,
            PROGRAM,
            3,
            RunLimits {
                events: 9,
                ..LIMITS
            },
        );
        assert_eq!((result.stop, result.tick), (StopReason::EventLimit, 8));
        assert_eq!(result.transactions[1].completed_at, Some(8));
        assert!(before_load_response.commits().is_empty());
        assert_eq!(before_load_response.registers()[3], 0);

        let mut before_exit_ack = Cpu::new(0, 32, 2);
        let result = program(
            &mut before_exit_ack,
            PROGRAM,
            3,
            RunLimits {
                ticks: 21,
                ..LIMITS
            },
        );
        assert_eq!((result.stop, result.tick), (StopReason::TickLimit, 21));
        assert_eq!(result.transactions[4].accepted_at, Some(18));
        assert_eq!(result.transactions[4].completed_at, None);
        assert_eq!(
            before_exit_ack
                .commits()
                .iter()
                .map(|commit| commit.at)
                .collect::<Vec<_>>(),
            [8, 13]
        );
        assert_eq!(before_exit_ack.pc(), 8);
    }

    #[test]
    fn addressing_faults_capture_instruction_pc_and_access_address() {
        // Fetch alignment, data alignment, and a routed data fault are delivered
        // at different stages of the instruction.
        for (entry, stack, tick, addr, reason, transactions) in [
            (2, 32, 0, 2, "Misaligned(Fetch)", 0),
            (0, 33, 5, 33, "Misaligned(Read)", 1),
            (0, 128, 6, 128, "AccessFault(Read): Unmapped", 2),
            (128, 32, 1, 128, "AccessFault(Fetch): Unmapped", 1),
        ] {
            let mut cpu = Cpu::new(entry, stack, 2);
            let result = program(&mut cpu, PROGRAM, 3, LIMITS);
            assert_eq!(result.tick, tick);
            assert_eq!(
                result.stop,
                StopReason::SourceStopped {
                    source: 0,
                    reason: reason.into()
                }
            );
            assert_eq!(result.transactions.len(), transactions);
            let fault = cpu.fault().unwrap();
            assert_eq!(
                (fault.pc, fault.addr, fault.reason.as_str()),
                (entry, Some(addr), reason)
            );
            assert!(fault.architectural.is_some());
            assert_eq!(cpu.pc(), entry);
            assert!(cpu.commits().is_empty());
        }
    }

    #[test]
    fn overflowing_execution_tick_stops_at_fetch_delivery() {
        let mut cpu = Cpu::new(0, 32, u64::MAX);
        let result = program(&mut cpu, PROGRAM, 3, LIMITS);
        assert_eq!(result.tick, 3);
        assert_eq!(
            result.stop,
            StopReason::SourceStopped {
                source: 0,
                reason: "execution tick overflow: 3 + 18446744073709551615".into(),
            }
        );
        assert_eq!(cpu.fault(), None);
        assert!(cpu.commits().is_empty());
    }
}
