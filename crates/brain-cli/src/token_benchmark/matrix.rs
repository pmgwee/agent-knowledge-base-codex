use super::{BenchmarkCondition, BenchmarkHarness, PlannedSample};

pub fn plan_matrix(task_ids: &[String], repeats: u32, seed: u64) -> Vec<PlannedSample> {
    let mut pairs = Vec::new();
    for (task_index, task_id) in task_ids.iter().enumerate() {
        for repeat in 1..=repeats {
            let harnesses = if (task_index + repeat as usize) % 2 == 0 {
                BenchmarkHarness::ALL
            } else {
                [BenchmarkHarness::Codex, BenchmarkHarness::ClaudeCode]
            };
            for harness in harnesses {
                pairs.push((task_id.clone(), harness, repeat));
            }
        }
    }
    let mut rng = DeterministicRng::new(seed);
    for index in (1..pairs.len()).rev() {
        let other = rng.index(index + 1);
        pairs.swap(index, other);
    }
    let mut samples = Vec::with_capacity(pairs.len() * 2);
    for (pair_index, (task_id, harness, repeat)) in pairs.into_iter().enumerate() {
        let first = if pair_index % 2 == 0 {
            BenchmarkCondition::BrainOff
        } else {
            BenchmarkCondition::BrainOn
        };
        let pair_id = format!("{task_id}-{}-r{repeat:02}", harness.as_str());
        for (order, condition) in [first, first.opposite()].into_iter().enumerate() {
            samples.push(PlannedSample {
                sample_id: format!("{pair_id}-{}", condition_name(condition)),
                pair_id: pair_id.clone(),
                task_id: task_id.clone(),
                harness,
                repeat,
                condition,
                order: order as u8,
            });
        }
    }
    samples
}

const fn condition_name(condition: BenchmarkCondition) -> &'static str {
    match condition {
        BenchmarkCondition::BrainOff => "off",
        BenchmarkCondition::BrainOn => "on",
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    pub(crate) const fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    pub(crate) fn index(&mut self, upper: usize) -> usize {
        (self.next_u64() % upper as u64) as usize
    }
}
