//! Test-only fault injection primitives. Production crates do not depend on this crate.

pub mod golden_flow;

use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FaultPoint {
    BeforeDurableCommit,
    AfterPrepareCommit,
    AfterPartWrite,
    AfterArtifactFlush,
    AfterAtomicRename,
    DuringCheckpointCommit,
    AfterCheckpointCommit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectedFault {
    point: FaultPoint,
}

impl InjectedFault {
    #[must_use]
    pub fn point(self) -> FaultPoint {
        self.point
    }
}

impl Display for InjectedFault {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "fault injected at {:?}", self.point)
    }
}

impl std::error::Error for InjectedFault {}

pub trait FaultInjector {
    fn hit(&mut self, point: FaultPoint) -> Result<(), InjectedFault>;
}

#[derive(Debug, Default)]
pub struct NoFaults;

impl FaultInjector for NoFaults {
    fn hit(&mut self, _point: FaultPoint) -> Result<(), InjectedFault> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct ScriptedFaults {
    armed: BTreeSet<FaultPoint>,
}

impl ScriptedFaults {
    #[must_use]
    pub fn once(points: impl IntoIterator<Item = FaultPoint>) -> Self {
        Self {
            armed: points.into_iter().collect(),
        }
    }
}

impl FaultInjector for ScriptedFaults {
    fn hit(&mut self, point: FaultPoint) -> Result<(), InjectedFault> {
        if self.armed.remove(&point) {
            Err(InjectedFault { point })
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FaultInjector, FaultPoint, NoFaults, ScriptedFaults};

    #[test]
    fn no_faults_never_interrupts() {
        let mut injector = NoFaults;
        assert_eq!(injector.hit(FaultPoint::AfterArtifactFlush), Ok(()));
    }

    #[test]
    fn scripted_fault_fires_only_once() {
        let point = FaultPoint::DuringCheckpointCommit;
        let mut injector = ScriptedFaults::once([point]);

        let fault = injector.hit(point).expect_err("the armed point must fail");
        assert_eq!(fault.point(), point);
        assert_eq!(injector.hit(point), Ok(()));
    }
}
