use core::cell::RefCell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use heapless::Deque;

use synth_core::{ControlMessage, LayerTarget, SequenceUpdate};

// Keep this small so the bounded in-place scan spends negligible time in its
// critical section even under an NRPN flood.
pub const CONTROL_QUEUE_CAPACITY: usize = 32;

pub struct ControlQueue {
    queue: Mutex<CriticalSectionRawMutex, RefCell<Deque<ControlMessage, CONTROL_QUEUE_CAPACITY>>>,
}

impl ControlQueue {
    pub const fn new() -> Self {
        Self {
            queue: Mutex::new(RefCell::new(Deque::new())),
        }
    }

    /// Enqueue a control, replacing an older queued update to the same
    /// parameter or modulation-route field when possible.
    pub fn try_send(
        &self,
        command: ControlMessage,
    ) -> Result<(), embassy_sync::channel::TrySendError<ControlMessage>> {
        self.queue.lock(|queue| {
            let mut queue = queue.borrow_mut();
            let replacement = queue.iter().enumerate().find_map(|(index, existing)| {
                (replaceable_same_field(existing, &command)
                    && replacement_preserves_edit_order(&queue, index, &command))
                .then_some(index)
            });
            if let Some(existing) = replacement.and_then(|index| queue.iter_mut().nth(index)) {
                *existing = command;
                return Ok(());
            }
            if queue.is_full() && is_topology_control(&command) {
                evict_oldest_parameter(&mut queue);
            }
            queue
                .push_back(command)
                .map_err(embassy_sync::channel::TrySendError::Full)
        })
    }

    pub fn try_receive(&self) -> Result<ControlMessage, embassy_sync::channel::TryReceiveError> {
        self.queue.lock(|queue| {
            queue
                .borrow_mut()
                .pop_front()
                .ok_or(embassy_sync::channel::TryReceiveError::Empty)
        })
    }
}

fn uses_edit_target(command: &ControlMessage) -> bool {
    matches!(
        command,
        ControlMessage::SetParam {
            target: LayerTarget::Edit,
            ..
        } | ControlMessage::SetModulationParam {
            target: LayerTarget::Edit,
            ..
        } | ControlMessage::SetSequence {
            target: LayerTarget::Edit,
            ..
        }
    )
}

fn replacement_preserves_edit_order(
    queue: &Deque<ControlMessage, CONTROL_QUEUE_CAPACITY>,
    existing_index: usize,
    incoming: &ControlMessage,
) -> bool {
    let later = queue.iter().skip(existing_index + 1);
    if uses_edit_target(incoming) {
        !later
            .into_iter()
            .any(|command| matches!(command, ControlMessage::SetEditLayer(_)))
    } else if matches!(incoming, ControlMessage::SetEditLayer(_)) {
        !later.into_iter().any(uses_edit_target)
    } else {
        true
    }
}

fn evict_oldest_parameter(queue: &mut Deque<ControlMessage, CONTROL_QUEUE_CAPACITY>) {
    let Some(index) = queue
        .iter()
        .position(|command| !is_topology_control(command))
    else {
        return;
    };

    // Rotate the selected entry to the front, remove it, then restore the
    // relative order of every retained command. Each push follows a pop, so it
    // cannot exceed the fixed capacity.
    for _ in 0..index {
        if let Some(command) = queue.pop_front() {
            let _ = queue.push_back(command);
        }
    }
    let _ = queue.pop_front();
    for _ in 0..queue.len().saturating_sub(index) {
        if let Some(command) = queue.pop_front() {
            let _ = queue.push_back(command);
        }
    }
}

fn replaceable_same_field(existing: &ControlMessage, incoming: &ControlMessage) -> bool {
    match (existing, incoming) {
        (
            ControlMessage::SetParam {
                target: left_target,
                param: left,
                ..
            },
            ControlMessage::SetParam {
                target: right_target,
                param: right,
                ..
            },
        ) => left_target == right_target && left == right,
        (
            ControlMessage::SetModulationParam {
                target: left_target,
                route: left_route,
                parameter: left_parameter,
            },
            ControlMessage::SetModulationParam {
                target: right_target,
                route: right_route,
                parameter: right_parameter,
            },
        ) => {
            left_target == right_target
                && left_route == right_route
                && core::mem::discriminant(left_parameter)
                    == core::mem::discriminant(right_parameter)
        }
        (
            ControlMessage::SetSequence {
                target: left_target,
                update: left,
            },
            ControlMessage::SetSequence {
                target: right_target,
                update: right,
            },
        ) => left_target == right_target && sequence_same_field(*left, *right),
        (ControlMessage::SetLayerMode(_), ControlMessage::SetLayerMode(_))
        | (ControlMessage::SetSplitPoint(_), ControlMessage::SetSplitPoint(_))
        | (ControlMessage::SetEditLayer(_), ControlMessage::SetEditLayer(_)) => true,
        _ => false,
    }
}

fn sequence_same_field(left: SequenceUpdate, right: SequenceUpdate) -> bool {
    match (left, right) {
        (SequenceUpdate::Type(_), SequenceUpdate::Type(_))
        | (SequenceUpdate::GatedMode(_), SequenceUpdate::GatedMode(_)) => true,
        (
            SequenceUpdate::GatedDestination { track: left, .. },
            SequenceUpdate::GatedDestination { track: right, .. },
        ) => left == right,
        (
            SequenceUpdate::GatedStep {
                track: left_track,
                step: left_step,
                ..
            },
            SequenceUpdate::GatedStep {
                track: right_track,
                step: right_step,
                ..
            },
        ) => left_track == right_track && left_step == right_step,
        (
            SequenceUpdate::PolyNote {
                step: left_step,
                lane: left_lane,
                ..
            },
            SequenceUpdate::PolyNote {
                step: right_step,
                lane: right_lane,
                ..
            },
        )
        | (
            SequenceUpdate::PolyVelocity {
                step: left_step,
                lane: left_lane,
                ..
            },
            SequenceUpdate::PolyVelocity {
                step: right_step,
                lane: right_lane,
                ..
            },
        )
        | (
            SequenceUpdate::PolyLaneStep {
                step: left_step,
                lane: left_lane,
                ..
            },
            SequenceUpdate::PolyLaneStep {
                step: right_step,
                lane: right_lane,
                ..
            },
        ) => left_step == right_step && left_lane == right_lane,
        _ => false,
    }
}

fn is_topology_control(command: &ControlMessage) -> bool {
    matches!(
        command,
        ControlMessage::SetLayerMode(_)
            | ControlMessage::SetSplitPoint(_)
            | ControlMessage::SetEditLayer(_)
    )
}

#[cfg(test)]
mod tests {
    use synth_core::{
        ControlMessage, GatedStep, LayerId, LayerMode, LayerTarget, ModDestination, ModRoute,
        ModSource, ModulationParam, ParamId, SequenceUpdate,
    };

    use super::{CONTROL_QUEUE_CAPACITY, ControlQueue};

    #[test]
    fn topology_control_survives_a_saturated_parameter_queue() {
        let queue = ControlQueue::new();
        assert!(
            queue
                .try_send(ControlMessage::SetLayerMode(LayerMode::Stack))
                .is_ok()
        );
        let parameters = [
            ModulationParam::Source(ModSource::Lfo1),
            ModulationParam::Destination(ModDestination::FilterCutoff),
            ModulationParam::Amount(0.5),
        ];
        let mut sent = 1;
        'fill: for index in 0..8 {
            for parameter in parameters {
                if sent == CONTROL_QUEUE_CAPACITY {
                    break 'fill;
                }
                assert!(
                    queue
                        .try_send(ControlMessage::SetModulationParam {
                            target: LayerTarget::Explicit(LayerId::A),
                            route: ModRoute::Free(index),
                            parameter,
                        })
                        .is_ok()
                );
                sent += 1;
            }
        }
        for index in 0..8 {
            if sent == CONTROL_QUEUE_CAPACITY {
                break;
            }
            assert!(
                queue
                    .try_send(ControlMessage::SetModulationParam {
                        target: LayerTarget::Explicit(LayerId::B),
                        route: ModRoute::Free(index),
                        parameter: ModulationParam::Amount(0.25),
                    })
                    .is_ok()
            );
            sent += 1;
        }
        assert_eq!(sent, CONTROL_QUEUE_CAPACITY);

        assert!(queue.try_send(ControlMessage::SetSplitPoint(72)).is_ok());
        assert!(
            queue
                .try_send(ControlMessage::SetEditLayer(LayerId::B))
                .is_ok()
        );

        let mut found_mode = false;
        let mut found_split = false;
        let mut found_edit = false;
        while let Ok(command) = queue.try_receive() {
            found_mode |= matches!(command, ControlMessage::SetLayerMode(LayerMode::Stack));
            found_split |= matches!(command, ControlMessage::SetSplitPoint(72));
            found_edit |= matches!(command, ControlMessage::SetEditLayer(LayerId::B));
        }
        assert!(found_mode);
        assert!(found_split);
        assert!(found_edit);
    }

    #[test]
    fn edit_target_updates_are_not_coalesced_across_layer_changes() {
        let queue = ControlQueue::new();
        assert!(
            queue
                .try_send(ControlMessage::edit_param(ParamId::FilterCutoff, 0.1))
                .is_ok()
        );
        assert!(
            queue
                .try_send(ControlMessage::SetEditLayer(LayerId::B))
                .is_ok()
        );
        assert!(
            queue
                .try_send(ControlMessage::edit_param(ParamId::FilterCutoff, 0.9))
                .is_ok()
        );
        assert!(
            queue
                .try_send(ControlMessage::SetEditLayer(LayerId::A))
                .is_ok()
        );

        assert!(matches!(
            queue.try_receive(),
            Ok(ControlMessage::SetParam {
                target: LayerTarget::Edit,
                value: 0.1,
                ..
            })
        ));
        assert!(matches!(
            queue.try_receive(),
            Ok(ControlMessage::SetEditLayer(LayerId::B))
        ));
        assert!(matches!(
            queue.try_receive(),
            Ok(ControlMessage::SetParam {
                target: LayerTarget::Edit,
                value: 0.9,
                ..
            })
        ));
        assert!(matches!(
            queue.try_receive(),
            Ok(ControlMessage::SetEditLayer(LayerId::A))
        ));
    }

    #[test]
    fn repeated_grid_drag_coalesces_to_one_sequence_command() {
        let queue = ControlQueue::new();
        for value in 0..=125 {
            assert!(
                queue
                    .try_send(ControlMessage::SetSequence {
                        target: LayerTarget::Explicit(LayerId::A),
                        update: SequenceUpdate::GatedStep {
                            track: 3,
                            step: 15,
                            value: GatedStep::Value(value),
                        },
                    })
                    .is_ok()
            );
        }
        assert!(matches!(
            queue.try_receive(),
            Ok(ControlMessage::SetSequence {
                update: SequenceUpdate::GatedStep {
                    track: 3,
                    step: 15,
                    value: GatedStep::Value(125),
                },
                ..
            })
        ));
        assert!(queue.try_receive().is_err());
    }
}
