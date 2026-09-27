//! Sliding-window minimum in O(1) amortised time with no allocations after construction.

pub struct SlidingMin {
    window: usize,
    values: Vec<f32>,
    stamps: Vec<u64>,
    head: usize,
    len: usize,
    time: u64,
}

impl SlidingMin {
    pub fn new(window: usize) -> Self {
        let window = window.max(1);
        Self {
            window,
            values: vec![0.0; window],
            stamps: vec![0; window],
            head: 0,
            len: 0,
            time: 0,
        }
    }

    pub fn reset(&mut self) {
        self.head = 0;
        self.len = 0;
        self.time = 0;
    }

    /// Pushes `value` and returns the minimum of the last `window` pushed values.
    pub fn push(&mut self, value: f32) -> f32 {
        let cap = self.window;
        // Drop entries that fall out of the window.
        while self.len > 0 && self.stamps[self.head] + self.window as u64 <= self.time {
            self.head = (self.head + 1) % cap;
            self.len -= 1;
        }
        // Drop entries that can never be the minimum again.
        while self.len > 0 && self.values[(self.head + self.len - 1) % cap] >= value {
            self.len -= 1;
        }
        let tail = (self.head + self.len) % cap;
        self.values[tail] = value;
        self.stamps[tail] = self.time;
        self.len += 1;
        self.time += 1;
        self.values[self.head]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_min(input: &[f32], window: usize, n: usize) -> f32 {
        let start = (n + 1).saturating_sub(window);
        input[start..=n].iter().copied().fold(f32::INFINITY, f32::min)
    }

    #[test]
    fn matches_naive_sliding_minimum() {
        let input: Vec<f32> = (0..500)
            .map(|n| (((n * 7919) % 1000) as f32 / 1000.0).sin())
            .collect();

        for window in [1, 2, 5, 17, 72] {
            let mut queue = SlidingMin::new(window);
            for (n, &x) in input.iter().enumerate() {
                assert_eq!(queue.push(x), naive_min(&input, window, n), "w={window} n={n}");
            }
        }
    }

    #[test]
    fn reset_forgets_previous_values() {
        let mut queue = SlidingMin::new(4);
        queue.push(0.1);
        queue.reset();

        assert_eq!(queue.push(0.9), 0.9);
    }
}
