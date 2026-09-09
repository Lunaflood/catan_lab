//! Resource flow estimates. A surplus can be exchanged only once, using the
//! rate of the resource GIVEN to the bank (not that of the missing resource).
use catan_core::action::Bundle;
use catan_core::board::{Board, PlayerId, RESOURCES};

pub fn exchange_rates(board: &Board, p: PlayerId) -> [f32; 5] {
    std::array::from_fn(|r| board.best_maritime_rate(p, RESOURCES[r]) as f32)
}

/// Fluid estimate in dice rolls, not an exact expected hitting time. One common
/// surplus pool covers all deficits, so the same wood cannot buy ore AND wheat.
pub fn build_time(hand: &Bundle, income: &[f32; 5], exchange: &[f32; 5], cost: &Bundle) -> f32 {
    let feasible = |t: f32| {
        let mut surplus = 0.0;
        let mut missing = 0.0;
        for r in 0..5 {
            let delta = hand[r] as f32 + income[r] * t - cost[r] as f32;
            if delta > 0.0 { surplus += delta / exchange[r]; }
            else { missing -= delta; }
        }
        surplus + 1e-6 >= missing
    };
    if feasible(0.0) { return 0.0; }
    if !feasible(512.0) { return f32::INFINITY; }
    let (mut lo, mut hi) = (0.0, 512.0);
    for _ in 0..18 {
        let mid = (lo + hi) * 0.5;
        if feasible(mid) { hi = mid; } else { lo = mid; }
    }
    hi
}
