//! Bounded UI delivery. Pixels remain in the authoritative framebuffer;
//! waiting stores only damage and one receipt, never encoded packet queues.
use ironrdp_pdu::geometry::InclusiveRectangle;
use serde::{Deserialize, Serialize};
use tokio::time::{Duration, Instant};
use uuid::Uuid;

use super::frame_batch::BatchPlan;

pub const STALL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameReceipt {
    pub generation: String,
    pub sequence: u32,
}

pub struct FrameDelivery {
    generation: String,
    sequence: u32,
    surface: Option<(u16, u16)>,
    pending: Option<BatchPlan>,
    flight: Option<(FrameReceipt, Instant)>,
    pub stalled: bool,
}

impl FrameDelivery {
    pub fn new() -> Self {
        Self {
            generation: Uuid::new_v4().to_string(),
            sequence: 0,
            surface: None,
            pending: None,
            flight: None,
            stalled: false,
        }
    }

    pub fn reset(&mut self) {
        self.generation = Uuid::new_v4().to_string();
        self.surface = None;
        self.pending = None;
        // The old transport slot remains occupied until its exact receipt
        // drains. It cannot acknowledge an image in the new generation.
    }

    pub fn damage(&mut self, surface: (u16, u16), mut plan: BatchPlan) {
        if self.surface != Some(surface) || plan.full_refresh {
            self.reset();
            self.surface = Some(surface);
            plan.full_refresh = true;
        }
        if let Some(previous) = self.pending.take() {
            plan.full_refresh |= previous.full_refresh;
            plan.rect.left = plan.rect.left.min(previous.rect.left);
            plan.rect.top = plan.rect.top.min(previous.rect.top);
            plan.rect.right = plan.rect.right.max(previous.rect.right);
            plan.rect.bottom = plan.rect.bottom.max(previous.rect.bottom);
        }
        self.pending = Some(plan);
    }

    pub fn take(&mut self, now: Instant) -> Option<(FrameReceipt, BatchPlan)> {
        if self.flight.is_some() || self.stalled {
            return None;
        }
        let plan = self.pending.take()?;
        self.sequence = self.sequence.wrapping_add(1);
        if self.sequence == 0 {
            self.generation = Uuid::new_v4().to_string();
            self.sequence = 1;
        }
        let receipt = FrameReceipt {
            generation: self.generation.clone(),
            sequence: self.sequence,
        };
        self.flight = Some((receipt.clone(), now));
        Some((receipt, plan))
    }

    pub fn acknowledge(&mut self, receipt: &FrameReceipt, refresh: bool) -> bool {
        if !self
            .flight
            .as_ref()
            .is_some_and(|(current, _)| current == receipt)
        {
            return false;
        }
        self.flight = None;
        if self.stalled || refresh {
            self.full_refresh();
        }
        self.stalled = false;
        true
    }

    fn full_refresh(&mut self) {
        if let Some((width, height)) = self.surface {
            self.pending = Some(BatchPlan {
                full_refresh: true,
                rect: InclusiveRectangle {
                    left: 0,
                    top: 0,
                    right: width - 1,
                    bottom: height - 1,
                },
            });
        }
    }

    pub fn check_stall(&mut self, now: Instant) -> bool {
        if !self.stalled
            && self
                .flight
                .as_ref()
                .is_some_and(|(_, sent)| now.duration_since(*sent) >= STALL_TIMEOUT)
        {
            self.stalled = true;
            return true;
        }
        false
    }

    pub fn deadline(&self) -> Option<Instant> {
        if self.stalled {
            None
        } else {
            self.flight.as_ref().map(|(_, sent)| *sent + STALL_TIMEOUT)
        }
    }

    pub fn failed_send(&mut self) {
        self.flight = None;
        self.stalled = false;
        self.full_refresh();
    }
}

pub async fn wait_for_stall(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn damage(left: u16, right: u16) -> BatchPlan {
        BatchPlan {
            full_refresh: false,
            rect: InclusiveRectangle {
                left,
                top: 0,
                right,
                bottom: 0,
            },
        }
    }
    #[test]
    fn delayed_ack_bounds_delivery_and_unions_all_damage() {
        let mut delivery = FrameDelivery::new();
        let now = Instant::now();
        delivery.damage((8, 1), damage(0, 7));
        let (first, _) = delivery.take(now).unwrap();
        for x in [0, 7, 2, 3] {
            delivery.damage((8, 1), damage(x, x));
            assert!(
                delivery.take(now).is_none(),
                "a pending packet must not replace the flight"
            );
        }
        assert!(delivery.acknowledge(&first, false));
        let (second, plan) = delivery.take(now).unwrap();
        assert_eq!(plan.rect, damage(0, 7).rect);
        assert!(!plan.full_refresh);
        assert!(!delivery.acknowledge(&first, false));
        assert_eq!(delivery.flight.as_ref().unwrap().0, second);
    }
    #[test]
    fn dropped_ack_stalls_at_five_seconds_and_recovers_full() {
        let mut delivery = FrameDelivery::new();
        let now = Instant::now();
        delivery.damage((8, 1), damage(0, 7));
        let (first, _) = delivery.take(now).unwrap();
        assert!(!delivery.check_stall(now + Duration::from_millis(4999)));
        assert!(delivery.check_stall(now + STALL_TIMEOUT));
        delivery.damage((8, 1), damage(2, 3));
        assert!(delivery.take(now).is_none());
        assert!(delivery.acknowledge(&first, false));
        assert!(delivery.take(now).unwrap().1.full_refresh);
    }
    #[test]
    fn reset_does_not_queue_a_second_packet_or_accept_old_ack_for_new_image() {
        let mut delivery = FrameDelivery::new();
        let now = Instant::now();
        delivery.damage((8, 1), damage(0, 7));
        let (old, _) = delivery.take(now).unwrap();
        delivery.reset(); // reconnect, including same-size reconnect
        delivery.damage((4, 1), damage(0, 3));
        assert!(delivery.take(now).is_none());
        assert!(delivery.acknowledge(&old, false)); // drain old slot only
        let (new, plan) = delivery.take(now).unwrap();
        assert_ne!(old.generation, new.generation);
        assert!(plan.full_refresh);
        assert!(!delivery.acknowledge(&old, true));
        assert_eq!(delivery.flight.as_ref().unwrap().0, new);
    }
}
