//! Звірка результатів повторних обчислень.

use std::collections::HashMap;

/// Кількість слів у відбитку результату одного завдання.
pub const DIGEST_WORDS: usize = 64;

/// Відбиток результату завдання.
pub type Digest = [u32; DIGEST_WORDS];

/// Результат звірки одного завдання.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    /// Завдання виконано вперше: запам'ятали результат як еталон.
    Reference,
    /// Результат збігся з еталоном.
    Match,
    /// Результат відрізняється: залізо помилилось або в цей раз, або в еталоні.
    Mismatch,
}

/// Порівнює результат кожного завдання з першим виконанням цього самого завдання.
#[derive(Debug, Default)]
pub struct Checker {
    references: HashMap<u32, Digest>,
    /// Завдань виконано.
    pub rounds: u64,
    /// Повторів, звірених з еталоном (без них тест нічого не перевірив).
    pub checked: u64,
    /// Розбіжностей.
    pub mismatches: u64,
}

impl Checker {
    /// Враховує результат завдання `seed`.
    pub fn observe(&mut self, seed: u32, digest: &Digest) -> Observed {
        self.rounds += 1;
        match self.references.get(&seed) {
            None => {
                self.references.insert(seed, *digest);
                Observed::Reference
            }
            Some(reference) => {
                self.checked += 1;
                if reference == digest {
                    Observed::Match
                } else {
                    self.mismatches += 1;
                    Observed::Mismatch
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_are_compared_with_the_first_result_of_the_same_task() {
        let mut c = Checker::default();
        let a = [1u32; DIGEST_WORDS];
        let b = [2u32; DIGEST_WORDS];
        assert_eq!(c.observe(10, &a), Observed::Reference);
        assert_eq!(c.observe(20, &b), Observed::Reference, "інше завдання");
        assert_eq!(c.checked, 0, "перші виконання нічого не перевіряють");
        assert_eq!(c.observe(10, &a), Observed::Match);
        let mut flipped = a;
        flipped[63] ^= 1;
        assert_eq!(c.observe(10, &flipped), Observed::Mismatch, "один біт");
        // Еталон не змінився: наступний правильний результат знову збігається.
        assert_eq!(c.observe(10, &a), Observed::Match);
        assert_eq!((c.rounds, c.checked, c.mismatches), (5, 3, 1));
    }
}
