//! Output-only SimpleUart target. Host display is outside the simulated device.
use crate::memory::{Fault, MemoryTarget, Operation, Request, ResponseResult, TargetCompletion};
use std::{cell::RefCell, num::NonZeroU64};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Byte {
    pub at: u64,
    pub value: u8,
}

pub struct SimpleUart<'a> {
    pub latency: NonZeroU64,
    pub output: &'a RefCell<Vec<Byte>>,
}

impl MemoryTarget for SimpleUart<'_> {
    fn select_lane(&self, offset: usize, request: &Request) -> Result<usize, Fault> {
        match &request.operation {
            Operation::Write { data } if offset == 0 && data.len() == 1 => Ok(0),
            _ => Err(Fault::InvalidMmioAccess),
        }
    }
    fn service_latency(&self, _: u64, _: usize, _: &Request) -> NonZeroU64 {
        self.latency
    }
    fn accepted(&mut self, at: u64, _: usize, request: &Request) {
        let Operation::Write { data } = &request.operation else {
            unreachable!()
        };
        self.output.borrow_mut().push(Byte { at, value: data[0] });
    }
    fn complete(&mut self, _: u64, _: usize, _: &Request) -> TargetCompletion {
        TargetCompletion::Respond(ResponseResult::WriteAck)
    }
}
