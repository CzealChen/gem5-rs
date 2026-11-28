//! Platform services and ROI accounting, independent of any CPU implementation.
use crate::m5::Operation;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub tick_femtoseconds: u64,
    pub instruction_budget: u64,
}

impl Config {
    pub fn validate(self) -> Result<Self, String> {
        if self.tick_femtoseconds == 0 || self.instruction_budget == 0 {
            return Err("time unit and instruction budget must be positive".into());
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Region {
    pub id: u64,
    pub begin_tick: u64,
    pub end_tick: u64,
    pub committed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Exit {
    pub code: u64,
    pub failed: bool,
}

pub struct Experiment {
    config: Config,
    active: Option<(u64, u64, u64)>,
    regions: Vec<Region>,
    exit: Option<Exit>,
}

impl Experiment {
    pub fn new(config: Config) -> Result<Self, String> {
        Ok(Self {
            config: config.validate()?,
            active: None,
            regions: Vec::new(),
            exit: None,
        })
    }
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }
    pub fn exit(&self) -> Option<Exit> {
        self.exit
    }
    pub fn budget(&self) -> u64 {
        self.config.instruction_budget
    }

    /// Called once at the irreversible commit boundary, before this instruction
    /// increments the committed count. No host-time or CPU-model dependencies.
    pub fn execute(
        &mut self,
        op: Operation,
        args: [u64; 2],
        tick: u64,
        before: u64,
    ) -> Result<u64, String> {
        if self.exit.is_some() {
            return Err("service after exit".into());
        }
        match op {
            Operation::Rpns => {
                let ns = u128::from(tick) * u128::from(self.config.tick_femtoseconds) / 1_000_000;
                return u64::try_from(ns).map_err(|_| "simulated nanoseconds overflow".into());
            }
            Operation::Exit | Operation::Fail => {
                if args[0] != 0 {
                    return Err("m5 exit/fail delay must be zero".into());
                }
                if self.active.is_some() {
                    return Err("exit with an unfinished ROI".into());
                }
                self.exit = Some(Exit {
                    code: if op == Operation::Fail { args[1] } else { 0 },
                    failed: op == Operation::Fail,
                });
            }
            Operation::WorkBegin => {
                if args[1] != 0 {
                    return Err("m5 threadid must be zero".into());
                }
                if self.active.is_some() {
                    return Err("nested ROI is unsupported".into());
                }
                self.active = Some((
                    args[0],
                    tick,
                    before.checked_add(1).ok_or("commit count overflow")?,
                ));
            }
            Operation::WorkEnd => {
                if args[1] != 0 {
                    return Err("m5 threadid must be zero".into());
                }
                let (id, begin_tick, begin_count) = self.active.ok_or("ROI end without begin")?;
                if id != args[0] {
                    return Err("ROI id mismatch".into());
                }
                let committed = before
                    .checked_sub(begin_count)
                    .ok_or("ROI commit count went backwards")?;
                if tick < begin_tick {
                    return Err("ROI tick went backwards".into());
                }
                self.regions.push(Region {
                    id,
                    begin_tick,
                    end_tick: tick,
                    committed,
                });
                self.active = None;
            }
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn experiment() -> Experiment {
        Experiment::new(Config {
            tick_femtoseconds: 1000,
            instruction_budget: 100,
        })
        .unwrap()
    }
    #[test]
    fn time_roi_and_full_width_failure() {
        let mut e = experiment();
        assert_eq!(e.execute(Operation::Rpns, [0, 0], 999, 0), Ok(0));
        assert_eq!(e.execute(Operation::Rpns, [0, 0], 1000, 0), Ok(1));
        e.execute(Operation::WorkBegin, [7, 0], 2000, 10).unwrap();
        e.execute(Operation::WorkEnd, [7, 0], 4000, 16).unwrap();
        assert_eq!(
            e.regions(),
            &[Region {
                id: 7,
                begin_tick: 2000,
                end_tick: 4000,
                committed: 5
            }]
        );
        e.execute(Operation::Fail, [0, u64::MAX], 5000, 17).unwrap();
        assert_eq!(
            e.exit(),
            Some(Exit {
                code: u64::MAX,
                failed: true
            })
        );
    }
    #[test]
    fn invalid_services_do_not_mutate_roi_or_exit() {
        let mut e = experiment();
        assert!(e.execute(Operation::Exit, [1, 0], 0, 0).is_err());
        assert!(e.execute(Operation::WorkBegin, [1, 1], 0, 0).is_err());
        assert!(e.execute(Operation::WorkEnd, [1, 0], 0, 0).is_err());
        e.execute(Operation::WorkBegin, [1, 0], 0, 0).unwrap();
        for (op, args) in [
            (Operation::WorkBegin, [1, 0]),
            (Operation::WorkEnd, [2, 0]),
            (Operation::Exit, [0, 0]),
        ] {
            assert!(e.execute(op, args, 1, 1).is_err());
        }
        assert_eq!(e.exit(), None);
        e.execute(Operation::WorkEnd, [1, 0], 1, 1).unwrap();
        assert_eq!(e.regions()[0].committed, 0);
    }
}
