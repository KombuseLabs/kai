//! The plan: which arm runs which task when, drawn from a seed.
//!
//! Ported from old kai (SplitMix64, blocks by repetition) and changed in one
//! place, the counterbalancing. A repetition is a block: every task once, in
//! a drawn order. Within a task the arms run in an order drawn for its first
//! repetition, and every later repetition rotates it by one place, so with
//! two arms the order alternates. Over the tasks the arm that is first in
//! the first repetition is spread evenly (the counts differ by at most
//! one), so each arm runs first about half the time whatever the number of
//! repetitions.
//!
//! The same seed gives the same plan on any build.

use std::collections::HashSet;

use types::study::Slot;

/// SplitMix64: small, fixed and version-independent, so that a seed
/// reproduces the same plan and the same resamples on any build.
pub struct Rng(u64);

impl Rng {
    /// A generator that starts from `seed`.
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (Lemire's rejection method); `n` must not be 0.
    pub fn below(&mut self, n: usize) -> usize {
        let n = n as u64;
        loop {
            let m = (self.next_u64() as u128) * (n as u128);
            if (m as u64) >= n.wrapping_neg() % n {
                return (m >> 64) as usize;
            }
        }
    }

    /// Puts `items` in a random order (Fisher-Yates).
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

/// Why a plan cannot be made.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlanError {
    /// A comparison needs two arms at least.
    #[error("a plan needs at least two arms, got {0}")]
    TooFewArms(usize),
    /// No task, nothing to plan.
    #[error("a plan needs at least one task")]
    NoTasks,
    /// No repetition, nothing to plan.
    #[error("a plan needs at least one repetition")]
    NoRepetitions,
    /// Two arms or two tasks with one name would share an identity.
    #[error("{kind} `{name}` is named twice")]
    Duplicate {
        /// `arm` or `task`.
        kind: &'static str,
        /// The repeated name.
        name: String,
    },
}

fn distinct<'a>(kind: &'static str, names: &'a [String]) -> Result<(), PlanError> {
    let mut seen: HashSet<&'a str> = HashSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(PlanError::Duplicate {
                kind,
                name: name.clone(),
            });
        }
    }
    Ok(())
}

/// The plan for `arms` and `tasks` over `repetitions` blocks, drawn from
/// `seed`: `tasks x arms x repetitions` slots, positions counted from 1.
pub fn build(
    arms: &[String],
    tasks: &[String],
    repetitions: u32,
    seed: u64,
) -> Result<Vec<Slot>, PlanError> {
    if arms.len() < 2 {
        return Err(PlanError::TooFewArms(arms.len()));
    }
    if tasks.is_empty() {
        return Err(PlanError::NoTasks);
    }
    if repetitions == 0 {
        return Err(PlanError::NoRepetitions);
    }
    distinct("arm", arms)?;
    distinct("task", tasks)?;

    let mut rng = Rng::new(seed);
    let n = arms.len();

    // Which arm opens each task's first repetition: as even as the numbers
    // allow over the tasks, then dealt out at random.
    let mut openers: Vec<usize> = (0..tasks.len()).map(|i| i % n).collect();
    rng.shuffle(&mut openers);
    // The arm order of each task's first repetition: a random order, turned
    // so that the dealt arm comes first.
    let first_orders: Vec<Vec<usize>> = openers
        .iter()
        .map(|&opener| {
            let mut order: Vec<usize> = (0..n).collect();
            rng.shuffle(&mut order);
            let at = order.iter().position(|&a| a == opener).unwrap_or(0);
            order.rotate_left(at);
            order
        })
        .collect();

    let mut plan = Vec::with_capacity(tasks.len() * n * repetitions as usize);
    let mut position = 0u32;
    for repetition in 1..=repetitions {
        let mut block: Vec<usize> = (0..tasks.len()).collect();
        rng.shuffle(&mut block);
        for task in block {
            let mut order = first_orders[task].clone();
            order.rotate_left((repetition as usize - 1) % n);
            for arm in order {
                position += 1;
                plan.push(Slot {
                    task: tasks[task].clone(),
                    arm: arms[arm].clone(),
                    repetition,
                    position,
                });
            }
        }
    }
    Ok(plan)
}

/// How often each arm ran first: for every (task, repetition), the arm of
/// the slot with the lowest position. In the order of `arms`; an arm that
/// never ran first has a count of 0, which here is a count, not an unknown.
pub fn first_counts(plan: &[Slot], arms: &[String]) -> Vec<(String, u32)> {
    let mut firsts: std::collections::HashMap<(&str, u32), &Slot> = Default::default();
    for slot in plan {
        let entry = firsts.entry((&slot.task, slot.repetition)).or_insert(slot);
        if slot.position < entry.position {
            *entry = slot;
        }
    }
    arms.iter()
        .map(|arm| {
            let count = firsts.values().filter(|s| &s.arm == arm).count();
            (arm.clone(), count as u32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn numbered(prefix: &str, n: usize) -> Vec<String> {
        (0..n).map(|i| format!("{prefix}{i}")).collect()
    }

    // Vectors from old kai (commit c483307, src/engine/plan.rs), produced by
    // running its generator: the port must give the same bits.
    #[test]
    fn splitmix64_reproduces_old_kai() {
        let first_four = |seed| {
            let mut rng = Rng::new(seed);
            [(); 4].map(|_| rng.next_u64())
        };
        assert_eq!(
            first_four(0),
            [
                16294208416658607535,
                7960286522194355700,
                487617019471545679,
                17909611376780542444
            ]
        );
        assert_eq!(
            first_four(1),
            [
                10451216379200822465,
                13757245211066428519,
                17911839290282890590,
                8196980753821780235
            ]
        );
        assert_eq!(
            first_four(7),
            [
                7191089600892374487,
                309689372594955804,
                16616101746815609346,
                10753165928301472203
            ]
        );
        assert_eq!(
            first_four(0xDEAD_BEEF),
            [
                5395234354446855067,
                16021672434157553954,
                153047824787635229,
                8387618351419058064
            ]
        );
    }

    #[test]
    fn below_and_shuffle_reproduce_old_kai() {
        let mut rng = Rng::new(1);
        let drawn: Vec<usize> = (0..12).map(|_| rng.below(5)).collect();
        assert_eq!(drawn, [2, 3, 4, 2, 2, 3, 4, 2, 1, 3, 2, 3]);
        let mut rng = Rng::new(7);
        let drawn: Vec<usize> = (0..6).map(|_| rng.below(1000)).collect();
        assert_eq!(drawn, [389, 16, 900, 582, 452, 249]);
        let mut items: Vec<u32> = (0..10).collect();
        Rng::new(7).shuffle(&mut items);
        assert_eq!(items, [9, 5, 8, 6, 1, 2, 4, 7, 0, 3]);
        let mut items: Vec<u32> = (0..10).collect();
        Rng::new(42).shuffle(&mut items);
        assert_eq!(items, [8, 3, 6, 5, 4, 0, 9, 2, 1, 7]);
    }

    #[test]
    fn below_stays_in_range() {
        let mut rng = Rng::new(1);
        let mut seen = [0u32; 5];
        for _ in 0..5000 {
            seen[rng.below(5)] += 1;
        }
        assert!(seen.iter().all(|&c| c > 800));
    }

    #[test]
    fn plan_is_deterministic_complete_and_blocked() {
        let arms = names(&["a", "b"]);
        let tasks = names(&["x", "y", "z"]);
        let first = build(&arms, &tasks, 3, 7).unwrap();
        assert_eq!(first, build(&arms, &tasks, 3, 7).unwrap());
        assert_ne!(first, build(&arms, &tasks, 3, 8).unwrap());
        assert_eq!(first.len(), 18);
        // Positions run 1..=18, and a repetition fills one block of six
        // consecutive positions.
        for (i, slot) in first.iter().enumerate() {
            assert_eq!(slot.position as usize, i + 1);
            assert_eq!(slot.repetition as usize, i / 6 + 1);
        }
        let mut ids: Vec<_> = first
            .iter()
            .map(|s| (s.task.clone(), s.arm.clone(), s.repetition))
            .collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 18);
        // Within a block each task's arms stand side by side.
        for pair in first.chunks(2) {
            assert_eq!(pair[0].task, pair[1].task);
            assert_ne!(pair[0].arm, pair[1].arm);
        }
    }

    #[test]
    fn repetitions_alternate_the_arm_order_of_a_task() {
        let arms = names(&["shadow", "ghost"]);
        let tasks = numbered("t", 7);
        let plan = build(&arms, &tasks, 4, 11).unwrap();
        for task in &tasks {
            let firsts: Vec<&str> = (1..=4)
                .map(|rep| {
                    plan.iter()
                        .filter(|s| &s.task == task && s.repetition == rep)
                        .min_by_key(|s| s.position)
                        .map(|s| s.arm.as_str())
                        .unwrap()
                })
                .collect();
            assert_ne!(firsts[0], firsts[1]);
            assert_eq!(firsts[0], firsts[2]);
            assert_eq!(firsts[1], firsts[3]);
        }
        // Even repetitions: each arm is first exactly half the time.
        for (_, count) in first_counts(&plan, &arms) {
            assert_eq!(count, 14);
        }
    }

    #[test]
    fn each_arm_goes_first_about_half_the_time() {
        let arms = names(&["shadow", "ghost"]);
        // One repetition, an odd number of tasks: the counts differ by one.
        let tasks = numbered("t", 9);
        for seed in 0..50 {
            let plan = build(&arms, &tasks, 1, seed).unwrap();
            let counts = first_counts(&plan, &arms);
            assert_eq!(counts[0].1 + counts[1].1, 9);
            assert!(counts[0].1.abs_diff(counts[1].1) == 1, "{counts:?}");
        }
        // Over many seeds the arm that opens the plan is a fair coin.
        let tasks = numbered("t", 4);
        let ghost_first = (0..400)
            .filter(|&seed| build(&arms, &tasks, 1, seed).unwrap()[0].arm == "ghost")
            .count();
        assert!((140..=260).contains(&ghost_first), "{ghost_first}");
    }

    #[test]
    fn three_arms_rotate_and_balance() {
        let arms = names(&["a", "b", "c"]);
        let tasks = numbered("t", 6);
        let plan = build(&arms, &tasks, 3, 5).unwrap();
        assert_eq!(plan.len(), 54);
        // Three repetitions rotate through all three orders: every arm is
        // first once per task.
        for (_, count) in first_counts(&plan, &arms) {
            assert_eq!(count, 6);
        }
    }

    #[test]
    fn a_plan_that_cannot_be_made_is_refused() {
        let two = names(&["a", "b"]);
        let one_task = names(&["x"]);
        assert_eq!(
            build(&names(&["a"]), &one_task, 1, 0),
            Err(PlanError::TooFewArms(1))
        );
        assert_eq!(build(&two, &[], 1, 0), Err(PlanError::NoTasks));
        assert_eq!(build(&two, &one_task, 0, 0), Err(PlanError::NoRepetitions));
        assert_eq!(
            build(&names(&["a", "a"]), &one_task, 1, 0),
            Err(PlanError::Duplicate {
                kind: "arm",
                name: "a".to_owned()
            })
        );
        assert!(matches!(
            build(&two, &names(&["x", "x"]), 1, 0),
            Err(PlanError::Duplicate { kind: "task", .. })
        ));
    }

    // A pinned plan: a change of the draw changes the order of every study
    // made from a seed, so it must be deliberate.
    const PINNED: [&str; 12] = [
        "ya1", "yb1", "zb1", "za1", "xa1", "xb1", "yb2", "ya2", "za2", "zb2", "xb2", "xa2",
    ];

    #[test]
    fn a_plan_for_a_seed_is_pinned() {
        let plan = build(&names(&["a", "b"]), &names(&["x", "y", "z"]), 2, 7).unwrap();
        let shown: Vec<String> = plan
            .iter()
            .map(|s| format!("{}{}{}", s.task, s.arm, s.repetition))
            .collect();
        assert_eq!(shown, PINNED);
    }
}
