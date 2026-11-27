//! What the next warps are likely to bring: the chance of a 5 star or of the featured one within so
//! many warps, the warps a copy costs on average and at worst, and how far a budget goes towards a
//! target like E2.
//!
//! The model and the method are hsr-optimizer's warp planner (fribbels/hsr-optimizer, MIT,
//! `src/lib/tabs/tabWarp/warpRates.ts` and `warpCalculatorController.ts`): exact probabilities, no
//! simulation.
//! - A character warp gives a 5 star at 0.6% a pull, and from the 74th pull 6% more each pull, so
//!   the 90th always does. A light cone warp: 0.8%, 7% more from the 66th, the 80th always does.
//! - The featured 5 star comes 56.25% of the time on a character warp's 50/50 (a lost 50/50 can
//!   still land on it, 1 in 8), 78.125% on a light cone warp's. Lost, the next one is guaranteed.
//! - Costs are distributions over the number of warps, the cost of several copies the
//!   convolution of each one's.

use super::Pool;

/// The odds of a pool's 5 stars
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rates {
    /// The chance of a 5 star on a pull before soft pity
    pub base: f64,
    /// The first pull (counting from 1) with a raised chance: it gets one `step` more than `base`,
    /// each pull after it another
    pub soft_from: u32,
    pub step: f64,
    /// The pull that always gives one
    pub hard: u32,
    /// The chance a 5 star is the featured one, when the last wasn't lost. None without a rate up
    pub featured: Option<f64>,
}

impl Rates {
    pub fn of(pool: Pool) -> Rates {
        match pool {
            Pool::LightCone | Pool::CollabLightCone => Rates {
                base: 0.008,
                soft_from: 66,
                step: 0.07,
                hard: pool.hard_pity(),
                featured: Some(0.78125),
            },
            Pool::Character | Pool::CollabCharacter => Rates {
                featured: Some(0.5625),
                hard: pool.hard_pity(),
                ..Self::STANDARD
            },
            Pool::Standard | Pool::Departure => Rates {
                hard: pool.hard_pity(),
                ..Self::STANDARD
            },
        }
    }

    const STANDARD: Rates = Rates {
        base: 0.006,
        soft_from: 74,
        step: 0.06,
        hard: 90,
        featured: None,
    };

    /// The chance the `n`th pull since the last 5 star (from 1) gives one, given none before it
    fn hazard(&self, n: u32) -> f64 {
        if n >= self.hard {
            return 1.0;
        }
        (self.base + f64::from((n + 1).saturating_sub(self.soft_from)) * self.step).min(1.0)
    }
}

/// A distribution over warps: `p[k]` is the chance it takes exactly `k`
pub type Cost = Vec<f64>;

/// What the next 5 star costs, `pity` pulls in
pub fn next_five(rates: &Rates, pity: u32) -> Cost {
    let mut cost = vec![0.0];
    let mut none_yet = 1.0;
    let mut n = pity;
    while none_yet > 1e-12 && n < rates.hard {
        n += 1;
        let p = rates.hazard(n);
        cost.push(none_yet * p);
        none_yet *= 1.0 - p;
    }
    cost
}

/// What the next featured 5 star costs
pub fn next_featured(rates: &Rates, pity: u32, guaranteed: bool) -> Cost {
    let first = next_five(rates, pity);
    let Some(win) = rates.featured.filter(|_| !guaranteed) else {
        return first;
    };
    let lose = convolve(&first, &next_five(rates, 0));
    let mut cost = vec![0.0; lose.len().max(first.len())];
    for (k, p) in first.iter().enumerate() {
        cost[k] += p * win;
    }
    for (k, p) in lose.iter().enumerate() {
        cost[k] += p * (1.0 - win);
    }
    cost
}

/// What `copies` featured 5 stars cost one after another. Each item is the cost up to and including
/// that copy
pub fn copies(rates: &Rates, pity: u32, guaranteed: bool, copies: usize) -> Vec<Cost> {
    let fresh = next_featured(rates, 0, false);
    let mut out = Vec::with_capacity(copies);
    let mut total = vec![1.0];
    for i in 0..copies {
        total = if i == 0 {
            convolve(&total, &next_featured(rates, pity, guaranteed))
        } else {
            convolve(&total, &fresh)
        };
        out.push(total.clone());
    }
    out
}

/// The chance it takes at most `warps`
pub fn within(cost: &Cost, warps: u32) -> f64 {
    cost.iter().take(warps as usize + 1).sum::<f64>().min(1.0)
}

pub fn expected(cost: &Cost) -> f64 {
    cost.iter().enumerate().map(|(k, p)| k as f64 * p).sum()
}

/// The most it can take
pub fn worst(cost: &Cost) -> u32 {
    cost.iter().rposition(|&p| p > 0.0).unwrap_or(0) as u32
}

fn convolve(a: &[f64], b: &[f64]) -> Cost {
    let mut out = vec![0.0; a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        if *x == 0.0 {
            continue;
        }
        for (j, y) in b.iter().enumerate() {
            out[i + j] += x * y;
        }
    }
    out
}

/// Warps from Stellar Jade and passes
pub fn warps_in_hand(jade: u32, passes: u32) -> u32 {
    passes + jade / 160
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, by: f64) {
        assert!((a - b).abs() <= by, "{a} isn't within {by} of {b}");
    }

    #[test]
    fn the_odds_match_hsr_optimizers_tables() {
        // warpRates.ts: the chance a run ends on each pull, from pity 0
        let character = next_five(&Rates::of(Pool::Character), 0);
        close(character[1], 0.006, 1e-12);
        close(character[74], 0.04253534861116301, 1e-12);
        close(character[77], 0.105347414622264, 1e-12);
        // Their last entry multiplies by an uncapped 102.6% (they renormalize later)
        close(character[89], 0.000002053394710113599, 1e-15);
        close(character.iter().sum(), 1.0, 1e-9);
        let light_cone = next_five(&Rates::of(Pool::LightCone), 0);
        close(light_cone[66], 0.046275731516717745, 1e-12);
        close(light_cone[79], 0.000005971634998864304, 1e-15);
    }

    #[test]
    fn budgets_and_costs_match_hsr_optimizers_tests() {
        let rates = Rates::of(Pool::Character);
        // 8000 jade and 176 passes, pity 62, E0 to E2 without refunds: 51.8%, guaranteed 67.3%
        let warps = warps_in_hand(8000, 176);
        close(within(&copies(&rates, 62, false, 3)[2], warps), 0.518, 0.002);
        close(within(&copies(&rates, 62, true, 3)[2], warps), 0.673, 0.002);
        // Guaranteed at pity 50, the first copy takes about 25, a fresh one about 90
        let costs = copies(&rates, 50, true, 2);
        close(expected(&costs[0]), 25.0, 3.0);
        close(expected(&costs[1]) - expected(&costs[0]), 90.0, 3.0);
    }

    #[test]
    fn the_worst_case_is_hard_pity_twice_on_a_fifty_fifty() {
        let rates = Rates::of(Pool::Character);
        assert_eq!(worst(&next_featured(&rates, 68, false)), 22 + 90);
        assert_eq!(worst(&next_featured(&rates, 68, true)), 22);
        assert_eq!(worst(&next_five(&Rates::of(Pool::LightCone), 79)), 1);
    }
}
