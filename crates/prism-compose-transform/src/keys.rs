//! Group keys (D5 of `compose-transform`): the first 64 bits of a BLAKE3 hash, chained from a
//! declaration path through the steps of the source structure, never from offsets, lines or
//! columns. Each key hashes its parent's key and its own step, a few dozen bytes, so a key costs
//! constant time whatever the nesting depth.

use prism_mir::{ClosureStep, GroupStep};

/// A tagged encoding hashed into a key: each part is written with its length, so no two lists
/// of parts encode alike.
struct KeyHasher(blake3::Hasher);

impl KeyHasher {
    fn new(tag: &str) -> Self {
        let mut hasher = KeyHasher(blake3::Hasher::new());
        hasher.text(tag);
        hasher
    }

    fn text(&mut self, text: &str) {
        self.0.update(&(text.len() as u64).to_le_bytes());
        self.0.update(text.as_bytes());
    }

    fn word(&mut self, word: u64) {
        self.0.update(&word.to_le_bytes());
    }

    fn finish(self) -> u64 {
        let hash = self.0.finalize();
        let mut first = [0u8; 8];
        first.copy_from_slice(&hash.as_bytes()[..8]);
        u64::from_le_bytes(first)
    }
}

/// The key of the body group of a composable function, and the root of every key written in
/// the top-level declaration `path` (`app/main::Counter`, `app/main::User.Avatar`).
pub fn body_key(path: &str) -> u64 {
    let mut hasher = KeyHasher::new("body");
    hasher.text(path);
    hasher.finish()
}

/// The key of a region of construct `step` in the region or body whose key is `parent`.
pub fn region_key(parent: u64, step: &GroupStep) -> u64 {
    let mut hasher = KeyHasher::new("region");
    hasher.word(parent);
    let (kind, ordinal, branch) = match *step {
        GroupStep::If { ordinal, branch } => ("if", ordinal, branch),
        GroupStep::When { ordinal, branch } => ("when", ordinal, branch),
        GroupStep::And { ordinal } => ("and", ordinal, 0),
        GroupStep::Or { ordinal } => ("or", ordinal, 0),
        GroupStep::Elvis { ordinal } => ("elvis", ordinal, 0),
        GroupStep::SafeCall { ordinal } => ("safe call", ordinal, 0),
        GroupStep::Loop { ordinal } => ("loop", ordinal, 0),
    };
    hasher.text(kind);
    hasher.word(u64::from(ordinal));
    hasher.word(u64::from(branch));
    hasher.finish()
}

/// The key of the body group of a composable closure reached by `step` from the region or
/// body whose key is `parent`.
pub fn closure_key(parent: u64, step: &ClosureStep) -> u64 {
    match step {
        ClosureStep::Argument {
            callee,
            ordinal,
            parameter,
        } => {
            let mut hasher = KeyHasher::new("closure");
            hasher.word(parent);
            hasher.text(callee);
            hasher.word(u64::from(*ordinal));
            hasher.word(u64::from(*parameter));
            hasher.finish()
        }
        ClosureStep::Value { ordinal } => {
            let mut hasher = KeyHasher::new("closure value");
            hasher.word(parent);
            hasher.word(u64::from(*ordinal));
            hasher.finish()
        }
    }
}

/// The key of the movable group of the `ordinal`-th `key` call of the region or body whose key
/// is `parent` (D13); the call's `Int` is its data key.
pub fn movable_key(parent: u64, ordinal: u32) -> u64 {
    let mut hasher = KeyHasher::new("key");
    hasher.word(parent);
    hasher.word(u64::from(ordinal));
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_depend_on_paths_and_steps_alone() {
        assert_eq!(body_key("app/main::Counter"), body_key("app/main::Counter"));
        assert_ne!(body_key("app/main::Counter"), body_key("app/main::Count"));
        let parent = body_key("app/main::Counter");
        let then = GroupStep::If {
            ordinal: 0,
            branch: 0,
        };
        assert_eq!(region_key(parent, &then), region_key(parent, &then));
        for other in [
            GroupStep::If {
                ordinal: 0,
                branch: 1,
            },
            GroupStep::If {
                ordinal: 1,
                branch: 0,
            },
            GroupStep::When {
                ordinal: 0,
                branch: 0,
            },
            GroupStep::Loop { ordinal: 0 },
        ] {
            assert_ne!(region_key(parent, &then), region_key(parent, &other));
        }
        assert_ne!(
            region_key(parent, &then),
            region_key(body_key("app/main::Other"), &then)
        );
        let argument = |ordinal, parameter| ClosureStep::Argument {
            callee: "app/main::Column".to_string(),
            ordinal,
            parameter,
        };
        assert_eq!(
            closure_key(parent, &argument(0, 0)),
            closure_key(parent, &argument(0, 0))
        );
        assert_ne!(
            closure_key(parent, &argument(0, 0)),
            closure_key(parent, &argument(1, 0))
        );
        assert_ne!(
            closure_key(parent, &argument(0, 0)),
            closure_key(parent, &argument(0, 1))
        );
        assert_ne!(
            closure_key(parent, &argument(0, 0)),
            closure_key(parent, &ClosureStep::Value { ordinal: 0 })
        );
        assert_ne!(movable_key(parent, 0), movable_key(parent, 1));
    }
}
