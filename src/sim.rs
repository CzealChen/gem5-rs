use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Pending events ordered by tick, then ascending phase, then insertion order.
pub struct Queue<E> {
    now: u64,
    next_sequence: u64,
    pending: BinaryHeap<Entry<E>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReason {
    Empty,
    TickLimit,
    EventLimit,
}

struct Entry<E> {
    key: (u64, u8, u64),
    event: E,
}

impl<E> Ord for Entry<E> {
    fn cmp(&self, other: &Self) -> Ordering {
        other.key.cmp(&self.key)
    }
}

impl<E> PartialOrd for Entry<E> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<E> PartialEq for Entry<E> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl<E> Eq for Entry<E> {}

impl<E> Default for Queue<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> Queue<E> {
    pub fn new() -> Self {
        Self {
            now: 0,
            next_sequence: 0,
            pending: BinaryHeap::new(),
        }
    }

    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn schedule(&mut self, at: u64, phase: u8, event: E) -> Result<(), String> {
        if at < self.now {
            return Err(format!(
                "event tick {at} precedes current tick {}",
                self.now
            ));
        }
        let sequence = self.next_sequence;
        self.next_sequence = sequence.checked_add(1).ok_or("event sequence exhausted")?;
        self.pending.push(Entry {
            key: (at, phase, sequence),
            event,
        });
        Ok(())
    }

    pub fn schedule_after(&mut self, delay: u64, phase: u8, event: E) -> Result<(), String> {
        let at = self
            .now
            .checked_add(delay)
            .ok_or_else(|| format!("event tick overflow: {} + {delay}", self.now))?;
        self.schedule(at, phase, event)
    }

    pub fn pop(&mut self) -> Option<(u64, E)> {
        let entry = self.pending.pop()?;
        self.now = entry.key.0;
        Some((self.now, entry.event))
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Dispatch up to `max_events` events through the inclusive tick `max_tick`.
    /// Each call starts a fresh event budget. The clock records the last popped
    /// tick, and pending events remain queued for the next call.
    pub fn run(
        &mut self,
        max_tick: u64,
        max_events: u64,
        mut dispatch: impl FnMut(&mut Self, u64, E) -> Result<(), String>,
    ) -> Result<StopReason, String> {
        let mut remaining = max_events;
        while let Some(next) = self.pending.peek() {
            if next.key.0 > max_tick {
                return Ok(StopReason::TickLimit);
            }
            if remaining == 0 {
                return Ok(StopReason::EventLimit);
            }
            let (tick, event) = self.pop().expect("a pending event was just observed");
            remaining -= 1;
            dispatch(self, tick, event)?;
        }
        Ok(StopReason::Empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_replays_ticks_phases_and_fifo_ties() {
        struct Event(&'static str);

        fn trace() -> Vec<(u64, &'static str)> {
            let mut queue = Queue::new();
            assert_eq!(queue.now(), 0);
            queue.schedule(9, 0, Event("late")).unwrap();
            queue.schedule(3, 2, Event("first tie")).unwrap();
            queue.schedule(3, 2, Event("second tie")).unwrap();
            queue.schedule(3, 1, Event("early phase")).unwrap();
            assert!(!queue.is_empty());

            let mut trace = Vec::new();
            while let Some((tick, Event(name))) = queue.pop() {
                assert_eq!(queue.now(), tick);
                trace.push((tick, name));
            }
            assert!(queue.is_empty());
            assert_eq!(queue.now(), 9);
            trace
        }

        let expected = [
            (3, "early phase"),
            (3, "first tie"),
            (3, "second tie"),
            (9, "late"),
        ];
        assert_eq!(trace(), expected);
        assert_eq!(trace(), expected);
    }

    #[test]
    fn scheduling_preserves_monotonic_time() {
        let mut queue = Queue::new();
        queue.schedule(7, 0, "advance").unwrap();
        assert_eq!(queue.pop(), Some((7, "advance")));
        assert!(queue.schedule(6, 0, "past").is_err());
        queue.schedule(7, 1, "same tick").unwrap();
        assert_eq!(queue.pop(), Some((7, "same tick")));
        assert!(queue.is_empty());
    }

    #[test]
    fn relative_scheduling_reaches_the_last_tick() {
        let mut queue = Queue::new();
        queue.schedule(u64::MAX - 2, 0, "advance").unwrap();
        assert_eq!(queue.pop(), Some((u64::MAX - 2, "advance")));
        assert!(queue.schedule_after(3, 0, "overflow").is_err());
        queue.schedule_after(2, 0, "last tick").unwrap();
        assert_eq!(queue.pop(), Some((u64::MAX, "last tick")));
        assert!(queue.is_empty());
    }

    #[test]
    fn run_applies_a_fresh_budget_to_each_batch() {
        let mut queue = Queue::new();
        queue.schedule(4, 0, 0).unwrap();
        let mut trace = Vec::new();
        let mut dispatch = |queue: &mut Queue<_>, tick, n| {
            trace.push((tick, n));
            queue.schedule_after(0, 0, n + 1)
        };

        assert_eq!(queue.run(4, 0, &mut dispatch), Ok(StopReason::EventLimit));
        assert_eq!(queue.now(), 0);
        assert_eq!(queue.run(4, 3, &mut dispatch), Ok(StopReason::EventLimit));
        assert_eq!(queue.now(), 4);
        assert_eq!(queue.run(4, 1, dispatch), Ok(StopReason::EventLimit));
        assert_eq!(trace, [(4, 0), (4, 1), (4, 2), (4, 3)]);
        assert_eq!(queue.pop(), Some((4, 4)));
    }

    #[test]
    fn run_dispatches_through_the_inclusive_tick_limit() {
        let mut queue = Queue::new();
        queue.schedule(8, 0, "late").unwrap();
        queue.schedule(3, 0, "timer").unwrap();
        let mut trace = Vec::new();
        let mut dispatch = |queue: &mut Queue<_>, tick, event| {
            trace.push((tick, event));
            if event == "timer" {
                queue.schedule_after(2, 0, "follow-up")?;
            }
            Ok(())
        };

        assert_eq!(queue.run(5, 10, &mut dispatch), Ok(StopReason::TickLimit));
        assert_eq!(queue.now(), 5);
        assert_eq!(queue.run(8, 1, dispatch), Ok(StopReason::Empty));
        assert_eq!(trace, [(3, "timer"), (5, "follow-up"), (8, "late")]);
        assert_eq!(queue.now(), 8);
    }
}
