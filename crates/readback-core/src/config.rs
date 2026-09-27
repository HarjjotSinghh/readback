//! Configuration for a [`crate::Readback`] instance.

use crate::lexicon::Locale;
use crate::omission::OmissionConfig;
use crate::policy::Policy;
use crate::redecode::RedecodeConfig;
use crate::suspicion::SuspicionConfig;
use serde::{Deserialize, Serialize};

/// How the risk number is assembled from stakes and evidence.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RiskWeights {
    /// Share of risk that applies regardless of stakes. Keeps a critical flag
    /// dangerous even in a low-stakes sentence.
    pub floor: f32,
}

impl Default for RiskWeights {
    fn default() -> Self {
        Self { floor: 0.4 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Language packs to load into the protected lexicon.
    #[serde(default = "default_locales")]
    pub locales: Vec<Locale>,
    /// Always-protected terms: names, projects, environments, jargon.
    #[serde(default)]
    pub vocabulary: Vec<String>,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub suspicion: SuspicionConfig,
    #[serde(default)]
    pub omission: OmissionConfig,
    #[serde(default)]
    pub redecode: RedecodeConfig,
    #[serde(default)]
    pub risk: RiskWeights,
}

fn default_locales() -> Vec<Locale> {
    vec![Locale::En]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            locales: default_locales(),
            vocabulary: Vec::new(),
            policy: Policy::default(),
            suspicion: SuspicionConfig::default(),
            omission: OmissionConfig::default(),
            redecode: RedecodeConfig::default(),
            risk: RiskWeights::default(),
        }
    }
}

impl Config {
    /// Default lexicon and thresholds, with the recommended per-app routing.
    pub fn recommended() -> Self {
        Self {
            policy: Policy::recommended(),
            ..Default::default()
        }
    }

    pub fn with_locales(mut self, locales: impl IntoIterator<Item = Locale>) -> Self {
        self.locales = locales.into_iter().collect();
        self
    }

    pub fn with_vocabulary<I, S>(mut self, terms: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.vocabulary.extend(terms.into_iter().map(Into::into));
        self
    }

    pub fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let config = Config::recommended().with_vocabulary(["Harpawan", "Recharge"]);
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&json).unwrap(), config);
    }

    #[test]
    fn an_empty_object_yields_defaults() {
        assert_eq!(
            serde_json::from_str::<Config>("{}").unwrap(),
            Config::default()
        );
    }
}
