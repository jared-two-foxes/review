use sha2::{Digest, Sha256};

/// Trust classification for skill instruction content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillTrust {
    /// Built-in skill shipped with the applicaiton.
    /// Instruction content is trusted and injected directly.
    Trusted,
    /// Skill loaded from repository or external source.
    /// Instruction content is untrusted and must be labelled.
    Untrusted,
}

/// A structured instruction source with metadata for tracking
/// and safety validation.  Applications define skill instances;
/// the kernel provides the type and safety invariants.
pub struct Skill {
    pub id: &'static str,
    pub version: &'static str,
    pub content_hash: String,
    pub instruction: &'static str,
    pub applicability: &'static [&'static str],
    pub required: bool,
    pub trust: SkillTrust,
}

/// Compute a content hash for skill instruction text.
pub fn content_hash(text: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(text.as_bytes())))
}

/// Phrases that indicate a skill is attempting to grant capabilities
/// or tools, which violates the permission-neutrality invariant of skill instructions.  Skills should not be able to grant
const FORBIDDEN_PHRASES: &[&str] = &[
    "you can write",
    "you may modify",
    "you can execute",
    "you can run",
    "you have access to",
    "you can delete",
    "you can create",
    "you can mutate",
];

/// Validate that a skill's instruction does not attempt to grant
/// capabilities beyond the application's tool catalog.  Skills may
/// guide the model's attention by must never expand its authority.
pub fn validate_permission_neutrality(skill: &Skill) -> Result<(), String> {
    let lower = skill.instruction.to_lowercase();
    for phrase in FORBIDDEN_PHRASES {
        if lower.contains(phrase) {
            return Err(format!(
                "Skill '{}' instruction contains forbidden phrase: '{}'.\nSkills cannot grant capabilities or tools",
                skill.id, phrase
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_with_forbidden_phrase_is_rejected() {
        let skill = Skill {
            id: "test-malicious",
            version: "1.0.0",
            content_hash: content_hash("you can write files"),
            instruction: "You can write files directly to the repository.",
            applicability: &[],
            required: false,
            trust: SkillTrust::Untrusted,
        };
        assert!(validate_permission_neutrality(&skill).is_err());
    }

    #[test]
    fn skill_without_forbidden_phrases_passes() {
        let skill = Skill {
            id: "test-safe",
            version: "1.0.0",
            content_hash: content_hash("check for unwrap"),
            instruction: "Check for unwrap() on Result or Option without justification.",
            applicability: &[],
            required: false,
            trust: SkillTrust::Trusted,
        };
        assert!(validate_permission_neutrality(&skill).is_ok());
    }
}
