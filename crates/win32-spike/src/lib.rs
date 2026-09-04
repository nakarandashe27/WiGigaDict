//! Isolated M0 Win32 risk-retirement harness.
//!
//! This crate is deliberately not linked into the desktop application. It records
//! transport evidence, never transcript content, and cannot promote a successful
//! Win32 call to `delivered` without target acknowledgement or a certified rule.

use serde::{Deserialize, Serialize};

#[cfg(windows)]
pub mod windows;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLevel {
    TargetAck,
    CertifiedTransport,
    TransportOnly,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Delivered,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    ClipboardBusy,
    ClipboardRestoreFailed,
    ElevatedTarget,
    FocusChanged,
    InputPartiallyAccepted,
    TargetMissing,
    UnsupportedCharacter,
    Win32CallFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertionMethod {
    UnicodePacket,
    VirtualKey,
    ClipboardPaste,
}

#[derive(Debug, Default)]
pub struct InsertionLadder {
    attempted: Vec<InsertionMethod>,
    stopped: bool,
}

impl InsertionLadder {
    const ORDER: [InsertionMethod; 3] = [
        InsertionMethod::UnicodePacket,
        InsertionMethod::VirtualKey,
        InsertionMethod::ClipboardPaste,
    ];

    #[must_use]
    pub fn next_method(&self) -> Option<InsertionMethod> {
        if self.stopped {
            return None;
        }
        Self::ORDER
            .iter()
            .copied()
            .find(|method| !self.attempted.contains(method))
    }

    pub fn record(&mut self, evidence: &MethodEvidence) {
        if self.stopped
            || self.attempted.contains(&evidence.method)
            || self.next_method() != Some(evidence.method)
        {
            self.stopped = true;
            return;
        }
        self.attempted.push(evidence.method);
        if !evidence.may_fallback() {
            self.stopped = true;
        }
    }

    #[must_use]
    pub fn attempted(&self) -> &[InsertionMethod] {
        &self.attempted
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MethodEvidence {
    pub method: InsertionMethod,
    pub expected_input_units: u32,
    pub accepted_input_units: u32,
    pub level: EvidenceLevel,
    pub status: DeliveryStatus,
    pub failure: Option<FailureCode>,
}

impl MethodEvidence {
    #[must_use]
    pub fn from_transport(
        method: InsertionMethod,
        expected_input_units: u32,
        accepted_input_units: u32,
        target_ack: bool,
        certified: bool,
    ) -> Self {
        let complete = expected_input_units == accepted_input_units;
        let (level, failure) = if !complete {
            (
                EvidenceLevel::None,
                Some(FailureCode::InputPartiallyAccepted),
            )
        } else if target_ack {
            (EvidenceLevel::TargetAck, None)
        } else if certified {
            (EvidenceLevel::CertifiedTransport, None)
        } else {
            (EvidenceLevel::TransportOnly, None)
        };
        let status = match level {
            EvidenceLevel::TargetAck | EvidenceLevel::CertifiedTransport => {
                DeliveryStatus::Delivered
            }
            EvidenceLevel::TransportOnly | EvidenceLevel::None => DeliveryStatus::Uncertain,
        };

        Self {
            method,
            expected_input_units,
            accepted_input_units,
            level,
            status,
            failure,
        }
    }

    #[must_use]
    pub fn failed(method: InsertionMethod, failure: FailureCode) -> Self {
        Self {
            method,
            expected_input_units: 0,
            accepted_input_units: 0,
            level: EvidenceLevel::None,
            status: DeliveryStatus::Uncertain,
            failure: Some(failure),
        }
    }

    #[must_use]
    pub fn may_fallback(&self) -> bool {
        self.accepted_input_units == 0
            && matches!(
                self.failure,
                Some(FailureCode::UnsupportedCharacter | FailureCode::ClipboardBusy)
            )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyTransition {
    Down,
    RepeatIgnored,
    Up,
    StrayUpIgnored,
}

#[derive(Debug, Default)]
pub struct HotkeyState {
    is_down: bool,
}

impl HotkeyState {
    pub fn observe(&mut self, is_down: bool) -> HotkeyTransition {
        match (self.is_down, is_down) {
            (false, true) => {
                self.is_down = true;
                HotkeyTransition::Down
            }
            (true, true) => HotkeyTransition::RepeatIgnored,
            (true, false) => {
                self.is_down = false;
                HotkeyTransition::Up
            }
            (false, false) => HotkeyTransition::StrayUpIgnored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_only_is_never_delivered() {
        let evidence =
            MethodEvidence::from_transport(InsertionMethod::UnicodePacket, 4, 4, false, false);

        assert_eq!(evidence.level, EvidenceLevel::TransportOnly);
        assert_eq!(evidence.status, DeliveryStatus::Uncertain);
        assert!(!evidence.may_fallback());
    }

    #[test]
    fn partial_input_is_uncertain_and_cannot_fallback() {
        let evidence =
            MethodEvidence::from_transport(InsertionMethod::VirtualKey, 4, 2, false, false);

        assert_eq!(evidence.level, EvidenceLevel::None);
        assert_eq!(evidence.status, DeliveryStatus::Uncertain);
        assert_eq!(evidence.failure, Some(FailureCode::InputPartiallyAccepted));
        assert!(!evidence.may_fallback());
    }

    #[test]
    fn only_strong_evidence_is_delivered() {
        for (target_ack, certified) in [(true, false), (false, true)] {
            let evidence = MethodEvidence::from_transport(
                InsertionMethod::UnicodePacket,
                2,
                2,
                target_ack,
                certified,
            );
            assert_eq!(evidence.status, DeliveryStatus::Delivered);
        }
    }

    #[test]
    fn hotkey_requires_one_physical_down_and_up() {
        let mut state = HotkeyState::default();

        assert_eq!(state.observe(true), HotkeyTransition::Down);
        assert_eq!(state.observe(true), HotkeyTransition::RepeatIgnored);
        assert_eq!(state.observe(false), HotkeyTransition::Up);
        assert_eq!(state.observe(false), HotkeyTransition::StrayUpIgnored);
    }

    #[test]
    fn ladder_never_falls_through_after_possible_delivery() {
        let mut ladder = InsertionLadder::default();
        let transport_only =
            MethodEvidence::from_transport(InsertionMethod::UnicodePacket, 2, 2, false, false);

        assert_eq!(ladder.next_method(), Some(InsertionMethod::UnicodePacket));
        ladder.record(&transport_only);
        assert_eq!(ladder.next_method(), None);
        assert_eq!(ladder.attempted(), &[InsertionMethod::UnicodePacket]);
    }

    #[test]
    fn ladder_falls_back_only_after_zero_unit_unsupported_method() {
        let mut ladder = InsertionLadder::default();
        let unsupported = MethodEvidence::failed(
            InsertionMethod::UnicodePacket,
            FailureCode::UnsupportedCharacter,
        );

        ladder.record(&unsupported);
        assert_eq!(ladder.next_method(), Some(InsertionMethod::VirtualKey));
    }
}
