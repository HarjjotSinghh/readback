//! Turning a risk number into an action, per destination app.
//!
//! The same shaky word means different things in a terminal and in a notes app.
//! Thresholds are *floors*: risk at or above `hold` holds, at or above
//! `highlight` highlights, otherwise it passes.

use crate::types::Action;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AppPolicy {
    /// Risk at or above this blocks insertion until the user confirms.
    pub hold: f32,
    /// Risk at or above this inserts, but marks the flagged spans.
    pub highlight: f32,
}

impl Default for AppPolicy {
    fn default() -> Self {
        Self {
            hold: 0.7,
            highlight: 0.35,
        }
    }
}

impl AppPolicy {
    /// For terminals and coding agents, where a dropped `don't` is an incident.
    pub fn paranoid() -> Self {
        Self {
            hold: 0.3,
            highlight: 0.15,
        }
    }

    /// For notes and drafts: still mark things, never block the user.
    ///
    /// Risk is clamped to `1.0`, so a hold threshold above that can never fire.
    /// A real number rather than an infinity, so the policy survives a JSON
    /// round trip.
    pub fn never_hold() -> Self {
        Self {
            hold: 1.01,
            highlight: 0.5,
        }
    }

    pub fn decide(&self, risk: f32) -> Action {
        if risk >= self.hold {
            Action::Hold
        } else if risk >= self.highlight {
            Action::Highlight
        } else {
            Action::Pass
        }
    }
}

/// A policy bound to a set of app names, written as `"Terminal|Cursor"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppRule {
    /// Pipe-separated app names, matched case-insensitively as substrings.
    pub pattern: String,
    #[serde(flatten)]
    pub policy: AppPolicy,
}

impl AppRule {
    pub fn new(pattern: impl Into<String>, policy: AppPolicy) -> Self {
        Self {
            pattern: pattern.into(),
            policy,
        }
    }

    pub fn matches(&self, app: &str) -> bool {
        let app = app.to_lowercase();
        self.pattern
            .split('|')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .any(|p| app.contains(&p.to_lowercase()))
    }
}

/// The full routing table: one default, plus per-app overrides.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub default: AppPolicy,
    #[serde(default)]
    pub apps: Vec<AppRule>,
}

impl Policy {
    /// Sensible starting point: paranoid in terminals and coding agents,
    /// standard in chat, non-blocking in notes.
    pub fn recommended() -> Self {
        Self {
            default: AppPolicy::default(),
            apps: vec![
                AppRule::new(
                    "Terminal|iTerm|Ghostty|Alacritty|Warp",
                    AppPolicy::paranoid(),
                ),
                AppRule::new("Claude|Cursor|Codex|Zed|Code", AppPolicy::paranoid()),
                AppRule::new("Slack|Discord|Teams|Gmail|Mail", AppPolicy::default()),
                AppRule::new("Notes|Obsidian|Bear|TextEdit", AppPolicy::never_hold()),
            ],
        }
    }

    /// First matching rule wins; order the list from most to least specific.
    pub fn for_app(&self, app: Option<&str>) -> AppPolicy {
        let Some(app) = app else { return self.default };
        self.apps
            .iter()
            .find(|rule| rule.matches(app))
            .map(|rule| rule.policy)
            .unwrap_or(self.default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_are_floors() {
        let p = AppPolicy::default();
        assert_eq!(p.decide(0.7), Action::Hold);
        assert_eq!(p.decide(0.35), Action::Highlight);
        assert_eq!(p.decide(0.34), Action::Pass);
    }

    #[test]
    fn notes_never_hold() {
        assert_eq!(AppPolicy::never_hold().decide(1.0), Action::Highlight);
    }

    #[test]
    fn app_patterns_match_case_insensitively() {
        let rule = AppRule::new("Terminal|Cursor", AppPolicy::paranoid());
        assert!(rule.matches("cursor"));
        assert!(rule.matches("Apple Terminal"));
        assert!(!rule.matches("Slack"));
    }

    #[test]
    fn unknown_apps_get_the_default() {
        let policy = Policy::recommended();
        assert_eq!(policy.for_app(Some("Figma")), AppPolicy::default());
        assert_eq!(policy.for_app(None), AppPolicy::default());
    }

    #[test]
    fn terminals_are_paranoid() {
        assert_eq!(
            Policy::recommended().for_app(Some("Ghostty")),
            AppPolicy::paranoid()
        );
    }
}
