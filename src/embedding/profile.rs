//! Model profiles (small/medium/large). A profile selects which embedder +
//! reranker from `MODEL_REGISTRY` Tome uses. The summariser is profile-
//! independent in this phase. The active profile is persisted in `meta`.

use crate::embedding::registry::{ModelEntry, lookup};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile { Small, Medium, Large }

impl Profile {
    pub const DEFAULT: Profile = Profile::Medium;
    pub const ALL: [Profile; 3] = [Profile::Small, Profile::Medium, Profile::Large];

    pub const fn as_str(self) -> &'static str {
        match self { Profile::Small => "small", Profile::Medium => "medium", Profile::Large => "large" }
    }
    /// Parse a tier string. Named `from_tier_str` (not `from_str`) to avoid
    /// shadowing/confusion with `std::str::FromStr` (S6).
    pub fn from_tier_str(s: &str) -> Option<Profile> {
        match s { "small" => Some(Profile::Small), "medium" => Some(Profile::Medium),
                  "large" => Some(Profile::Large), _ => None }
    }
    const fn embedder_name(self) -> &'static str {
        match self {
            Profile::Small  => "bge-small-en-v1.5",
            Profile::Medium => "bge-base-en-v1.5",
            Profile::Large  => "bge-large-en-v1.5",
        }
    }
    const fn reranker_name(self) -> &'static str {
        match self {
            Profile::Small  => "bge-reranker-base",
            Profile::Medium => "bge-reranker-large",
            Profile::Large  => "bge-reranker-v2-m3",
        }
    }
}

pub fn embedder_for(p: Profile) -> &'static ModelEntry {
    lookup(p.embedder_name()).expect("profile embedder must be registered in MODEL_REGISTRY")
}
pub fn reranker_for(p: Profile) -> &'static ModelEntry {
    lookup(p.reranker_name()).expect("profile reranker must be registered in MODEL_REGISTRY")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::registry::ModelKind;

    #[test]
    fn every_profile_resolves_to_registered_models_of_correct_kind() {
        for p in Profile::ALL {
            assert_eq!(embedder_for(p).kind, ModelKind::Embedder, "{:?} embedder", p);
            assert_eq!(reranker_for(p).kind, ModelKind::Reranker, "{:?} reranker", p);
        }
    }
    #[test]
    fn str_round_trips() {
        for p in Profile::ALL { assert_eq!(Profile::from_tier_str(p.as_str()), Some(p)); }
        assert_eq!(Profile::from_tier_str("xl"), None);
        assert_eq!(Profile::DEFAULT, Profile::Medium);
    }
}
