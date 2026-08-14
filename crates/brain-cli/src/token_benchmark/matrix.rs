use super::{BenchmarkCondition, BenchmarkHarness, PlannedSample};

pub fn plan_matrix(task_ids: &[String], repeats: u32, seed: u64) -> Vec<PlannedSample> {
    let mut blocks = Vec::new();
    for (task_index, task_id) in task_ids.iter().enumerate() {
        for repeat in 1..=repeats {
            let harnesses = if (task_index + repeat as usize) % 2 == 0 {
                BenchmarkHarness::ALL
            } else {
                [BenchmarkHarness::Codex, BenchmarkHarness::ClaudeCode]
            };
            for harness in harnesses {
                blocks.push((task_id.clone(), harness, repeat));
            }
        }
    }
    let mut rng = DeterministicRng::new(seed);
    for index in (1..blocks.len()).rev() {
        let other = rng.index(index + 1);
        blocks.swap(index, other);
    }
    let mut samples = Vec::with_capacity(blocks.len() * BenchmarkCondition::ALL.len());
    let mut latin_base = BenchmarkCondition::ALL;
    for (block_index, (task_id, harness, repeat)) in blocks.into_iter().enumerate() {
        if block_index % BenchmarkCondition::ALL.len() == 0 {
            shuffle(&mut latin_base, &mut rng);
        }
        let rotation = block_index % BenchmarkCondition::ALL.len();
        let block_id = format!("{task_id}-{}-r{repeat:02}", harness.as_str());
        for order in 0..BenchmarkCondition::ALL.len() {
            let condition = latin_base[(order + rotation) % BenchmarkCondition::ALL.len()];
            samples.push(PlannedSample {
                sample_id: format!("{block_id}-{}", condition.as_str()),
                pair_id: block_id.clone(),
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

fn shuffle<T>(values: &mut [T], rng: &mut DeterministicRng) {
    for index in (1..values.len()).rev() {
        let other = rng.index(index + 1);
        values.swap(index, other);
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
