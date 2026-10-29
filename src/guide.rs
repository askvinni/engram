//! Workflow guides served by `engram guide`. The installed skills are thin stubs
//! that call this, so skill content always matches the installed binary.

pub const LEARN: &str = include_str!("../guides/learn.md");
pub const MEMORY: &str = include_str!("../guides/memory.md");
pub const MEMORY_QUALITY: &str = include_str!("../guides/memory-quality.md");

#[derive(Debug)]
pub struct Topic {
    pub name: &'static str,
    pub description: &'static str,
    pub content: &'static str,
}

pub const TOPICS: &[Topic] = &[
    Topic {
        name: "plan",
        description: "Write and open an engram-plan issue",
        content: include_str!("../guides/plan.md"),
    },
    Topic {
        name: "objective",
        description: "Write and open an engram-objective issue for multi-plan goals",
        content: include_str!("../guides/objective.md"),
    },
    Topic {
        name: "learn",
        description: "Run engram plan learn/land and review the synthesized memory",
        content: LEARN,
    },
    Topic {
        name: "memory-quality",
        description: "The quality bar for what belongs in .engram/memory/",
        content: MEMORY_QUALITY,
    },
    Topic {
        name: "memory",
        description: "Navigate, evaluate, and compact .engram/memory/",
        content: MEMORY,
    },
    Topic {
        name: "dispatch",
        description: "Dispatch plans or objectives to background Claude sessions",
        content: include_str!("../guides/dispatch.md"),
    },
];

/// The topic list, one `name  description` line per topic.
pub fn list() -> String {
    let width = TOPICS.iter().map(|t| t.name.len()).max().unwrap_or(0);
    TOPICS
        .iter()
        .map(|t| format!("  {:width$}  {}\n", t.name, t.description))
        .collect()
}

/// The guide for `topic`, headed by the engram version that serves it.
pub fn render(topic: &str) -> anyhow::Result<String> {
    let Some(t) = TOPICS.iter().find(|t| t.name == topic) else {
        anyhow::bail!("unknown guide topic '{topic}'. Topics:\n{}", list());
    };
    Ok(format!(
        "<!-- engram {} · engram guide {} -->\n{}",
        env!("CARGO_PKG_VERSION"),
        t.name,
        t.content
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_heads_guide_with_version() {
        let out = render("plan").unwrap();
        let first = out.lines().next().unwrap();
        assert!(first.contains(env!("CARGO_PKG_VERSION")));
        assert!(first.contains("engram guide plan"));
        assert!(out.contains("# engram plan guide"));
    }

    #[test]
    fn render_unknown_topic_lists_topics() {
        let err = render("nope").unwrap_err().to_string();
        for t in TOPICS {
            assert!(err.contains(t.name), "missing {}", t.name);
        }
    }

    #[test]
    fn list_has_a_line_per_topic() {
        let out = list();
        assert_eq!(out.lines().count(), TOPICS.len());
        for t in TOPICS {
            assert!(out.contains(t.name), "missing {}", t.name);
        }
    }

    #[test]
    fn guides_do_not_rely_on_skill_arguments() {
        for t in TOPICS {
            assert!(
                !t.content.contains("$ARGUMENTS"),
                "{} uses $ARGUMENTS",
                t.name
            );
        }
    }
}
