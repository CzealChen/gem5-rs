//! Single-hart machine timer: read-only mtime at +0, mtimecmp at +8.
//! Selectable simulation-tick or host monotonic clock. The interrupt level
//! is sampled at instruction boundaries (this core has no WFI/idle state).
use crate::memory::{Fault, MemoryTarget, Operation, Request, ResponseResult, TargetCompletion};
use std::{
    cell::RefCell,
    num::NonZeroU64,
    rc::Rc,
    time::{Duration, Instant},
};

pub type Line = Rc<RefCell<Timer>>;

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClockConfig {
    Simulated { tick_period: u64 },
    Wall { frequency_hz: u64 },
}

impl ClockConfig {
    pub fn validate(self) -> Result<Self, String> {
        let value = match self {
            Self::Simulated { tick_period } => tick_period,
            Self::Wall { frequency_hz } => frequency_hz,
        };
        if value == 0 {
            Err("timer clock period/frequency must be positive".into())
        } else {
            Ok(self)
        }
    }
    pub fn build(self) -> Timer {
        match self {
            Self::Simulated { tick_period } => {
                Timer::new(NonZeroU64::new(tick_period).expect("validated period"))
            }
            Self::Wall { frequency_hz } => Timer {
                clock: Clock::Wall {
                    frequency_hz: NonZeroU64::new(frequency_hz).expect("validated frequency"),
                    start: Instant::now(),
                },
                compare: u64::MAX,
            },
        }
    }
}

enum Clock {
    Simulated(NonZeroU64),
    Wall {
        frequency_hz: NonZeroU64,
        start: Instant,
    },
}

fn wall_count(elapsed: Duration, frequency_hz: NonZeroU64) -> u64 {
    let count = elapsed
        .as_nanos()
        .saturating_mul(u128::from(frequency_hz.get()))
        / 1_000_000_000;
    count.min(u128::from(u64::MAX)) as u64
}

pub struct Timer {
    clock: Clock,
    compare: u64,
}

impl Timer {
    pub fn new(tick_period: NonZeroU64) -> Self {
        Self {
            clock: Clock::Simulated(tick_period),
            compare: u64::MAX,
        }
    }
    pub fn reset_wall_clock(&mut self) {
        if let Clock::Wall { start, .. } = &mut self.clock {
            *start = Instant::now();
        }
    }
    pub fn time(&self, at: u64) -> u64 {
        match self.clock {
            Clock::Simulated(period) => at / period.get(),
            Clock::Wall {
                frequency_hz,
                start,
            } => wall_count(start.elapsed(), frequency_hz),
        }
    }
    pub fn pending(&self, at: u64) -> bool {
        self.time(at) >= self.compare
    }
}

pub struct Mmio {
    pub timer: Line,
    pub latency: NonZeroU64,
}

impl MemoryTarget for Mmio {
    fn select_lane(&self, offset: usize, request: &Request) -> Result<usize, Fault> {
        match &request.operation {
            Operation::Read { size: 8 } if matches!(offset, 0 | 8) => Ok(0),
            Operation::Write { data } if offset == 8 && data.len() == 8 => Ok(0),
            _ => Err(Fault::InvalidMmioAccess),
        }
    }
    fn service_latency(&self, _: u64, _: usize, _: &Request) -> NonZeroU64 {
        self.latency
    }
    fn complete(&mut self, at: u64, offset: usize, request: &Request) -> TargetCompletion {
        let mut timer = self.timer.borrow_mut();
        TargetCompletion::Respond(match &request.operation {
            Operation::Read { .. } => ResponseResult::ReadData(
                (if offset == 0 {
                    timer.time(at)
                } else {
                    timer.compare
                })
                .to_le_bytes()
                .to_vec(),
            ),
            Operation::Write { data } => {
                timer.compare =
                    u64::from_le_bytes(data.as_slice().try_into().expect("validated width"));
                ResponseResult::WriteAck
            }
            _ => unreachable!("validated access"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(operation: Operation) -> Request {
        Request {
            source: 0,
            id: 0,
            address: 0,
            operation,
        }
    }

    #[test]
    fn mmio_clock_deadline_rearm_and_access_widths() {
        let line = Rc::new(RefCell::new(Timer::new(NonZeroU64::new(10).unwrap())));
        let mut mmio = Mmio {
            timer: line.clone(),
            latency: NonZeroU64::new(3).unwrap(),
        };
        let read = request(Operation::Read { size: 8 });
        assert!(!line.borrow().pending(100));
        let TargetCompletion::Respond(ResponseResult::ReadData(bytes)) =
            mmio.complete(129, 0, &read)
        else {
            panic!("read mtime");
        };
        assert_eq!(u64::from_le_bytes(bytes.try_into().unwrap()), 12);
        let write = request(Operation::Write {
            data: 15_u64.to_le_bytes().to_vec(),
        });
        assert_eq!(mmio.select_lane(8, &write), Ok(0));
        assert_eq!(mmio.select_lane(0, &write), Err(Fault::InvalidMmioAccess));
        mmio.complete(130, 8, &write);
        assert!(!line.borrow().pending(149));
        assert!(line.borrow().pending(150));
        mmio.complete(
            160,
            8,
            &request(Operation::Write {
                data: 100_u64.to_le_bytes().to_vec(),
            }),
        );
        assert!(!line.borrow().pending(160));
        for op in [
            Operation::Fetch { size: 8 },
            Operation::Read { size: 4 },
            Operation::Write { data: vec![0; 4] },
        ] {
            assert_eq!(
                mmio.select_lane(8, &request(op)),
                Err(Fault::InvalidMmioAccess)
            );
        }
        assert_eq!(mmio.select_lane(1, &read), Err(Fault::InvalidMmioAccess));
    }

    #[test]
    fn wall_frequency_uses_monotonic_elapsed_time_not_simulation_ticks() {
        assert_eq!(
            wall_count(
                Duration::from_micros(1234),
                NonZeroU64::new(1_000_000).unwrap()
            ),
            1234
        );
        assert_eq!(
            wall_count(
                Duration::from_nanos(999),
                NonZeroU64::new(1_000_000).unwrap()
            ),
            0
        );
        assert_eq!(
            wall_count(
                Duration::from_secs(u64::MAX),
                NonZeroU64::new(u64::MAX).unwrap()
            ),
            u64::MAX
        );
        let timer = ClockConfig::Wall {
            frequency_hz: 1_000_000,
        }
        .build();
        let before = timer.time(u64::MAX);
        std::thread::sleep(Duration::from_millis(2));
        assert!(timer.time(0) >= before + 1000);
        assert!(ClockConfig::Wall { frequency_hz: 0 }.validate().is_err());
        assert!(
            ClockConfig::Simulated { tick_period: 0 }
                .validate()
                .is_err()
        );
    }
}
