use std::sync::atomic::{AtomicU8, Ordering};

const RUNNING: u8 = 0;
const DRAINING: u8 = 1;
const FINISHED: u8 = 2;

#[derive(Default)]
pub(crate) struct ExitGate(AtomicU8);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExitAction {
    StartShutdown,
    Wait,
    Exit,
}

impl ExitGate {
    pub(crate) fn request(&self) -> ExitAction {
        match self
            .0
            .compare_exchange(RUNNING, DRAINING, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => ExitAction::StartShutdown,
            Err(FINISHED) => ExitAction::Exit,
            Err(_) => ExitAction::Wait,
        }
    }

    pub(crate) fn finish(&self) {
        self.0.store(FINISHED, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn repeated_quit_requests_cannot_exit_before_shutdown_finishes() {
        let gate = ExitGate::default();
        assert_eq!(gate.request(), ExitAction::StartShutdown);
        for _ in 0..10 {
            assert_eq!(gate.request(), ExitAction::Wait);
        }
        gate.finish();
        for _ in 0..10 {
            assert_eq!(gate.request(), ExitAction::Exit);
        }
    }

    #[test]
    fn concurrent_exit_requests_start_only_one_shutdown() {
        let gate = Arc::new(ExitGate::default());
        let barrier = Arc::new(Barrier::new(8));
        let requests: Vec<_> = (0..8)
            .map(|_| {
                let gate = gate.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    gate.request()
                })
            })
            .collect();
        let actions: Vec<_> = requests.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(
            actions
                .iter()
                .filter(|a| **a == ExitAction::StartShutdown)
                .count(),
            1
        );
        assert!(!actions.contains(&ExitAction::Exit));
    }
}
