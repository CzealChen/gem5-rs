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
}

pub trait Observer {
    fn retired(&mut self, retirement: &Retirement) -> Result<(), String>;
    fn faulted(&mut self, _fault: &crate::riscv::Fault) -> Result<(), String> {
        Ok(())
    }
}

pub struct Shared<T>(pub std::rc::Rc<std::cell::RefCell<T>>);

impl<T: Observer> Observer for Shared<T> {
    fn retired(&mut self, retirement: &Retirement) -> Result<(), String> {
        self.0.borrow_mut().retired(retirement)
    }
    fn faulted(&mut self, fault: &crate::riscv::Fault) -> Result<(), String> {
        self.0.borrow_mut().faulted(fault)
    }
}
