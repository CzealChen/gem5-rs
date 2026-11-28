//! Read-only architectural observations. Consumers cannot mutate CPU state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryEffect {
    pub address: u64,
    pub write: bool,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Retirement {
    pub pc: u64,
    pub instruction: u32,
    pub next_pc: u64,
    pub registers: [u64; 32],
    pub memory: Option<MemoryEffect>,
    /// Present only after a supported enabled service completed successfully.
    pub service: Option<crate::m5::Operation>,
    pub machine: Option<crate::machine::Snapshot>,
}

#[derive(Clone, Debug)]
pub struct Trap {
    pub pc: u64,
    pub next_pc: u64,
    pub registers: [u64; 32],
    pub machine: crate::machine::Snapshot,
}

pub trait Observer {
    /// External timer level sampled before executing the next instruction.
    fn timer_pending(&mut self, _pending: bool) -> Result<(), String> {
        Ok(())
    }
    /// Synchronous exception or interrupt entry; neither retires an instruction.
    fn trapped(&mut self, _trap: &Trap) -> Result<(), String> {
        Ok(())
    }

    fn retired(&mut self, retirement: &Retirement) -> Result<(), String>;
    fn faulted(&mut self, _fault: &crate::riscv::Fault) -> Result<(), String> {
        Ok(())
    }
}

pub struct Shared<T>(pub std::rc::Rc<std::cell::RefCell<T>>);

impl<T: Observer> Observer for Shared<T> {
    fn timer_pending(&mut self, pending: bool) -> Result<(), String> {
        self.0.borrow_mut().timer_pending(pending)
    }
    fn trapped(&mut self, trap: &Trap) -> Result<(), String> {
        self.0.borrow_mut().trapped(trap)
    }

    fn retired(&mut self, retirement: &Retirement) -> Result<(), String> {
        self.0.borrow_mut().retired(retirement)
    }
    fn faulted(&mut self, fault: &crate::riscv::Fault) -> Result<(), String> {
        self.0.borrow_mut().faulted(fault)
    }
}
