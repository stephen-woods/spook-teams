use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::db::task::TaskStatus;
use crate::db::{self, Db};

// ── Parsed task representation ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedTask {
    pub source_id: String,
    pub title: String,
    pub section: String,
    pub done: bool,
}

// ── Parser (4.1) ──────────────────────────────────────────────────────────────

/// Parse an OpenSpec `tasks.md` file into a list of `ParsedTask`.
///
/// Format:
/// ```text
/// ## 1. Section Title
///
/// - [ ] 1.1 Task description
/// - [x] 1.2 Completed task
/// ```
pub fn parse_tasks_md(content: &str) -> Vec<ParsedTask> {
    let mut tasks = Vec::new();
    let mut current_section = String::new();
    let mut section_num: u32 = 0;
    let mut item_num: u32 = 0;

    for line in content.lines() {
        let line = line.trim();

        // Section heading: ## 1. Title
        if let Some(rest) = line.strip_prefix("## ") {
            current_section = rest.to_string();
            // Extract section number from "1. Title" or "1 Title"
            if let Some(dot_pos) = rest.find('.') {
                if let Ok(n) = rest[..dot_pos].trim().parse::<u32>() {
                    section_num = n;
                    item_num = 0;
                }
            }
            continue;
        }

        // Checkbox item: - [ ] or - [x]
        if let Some(rest) = line.strip_prefix("- [ ] ") {
            item_num += 1;
            let source_id = format!("{}.{}", section_num, item_num);
            tasks.push(ParsedTask {
                source_id,
                title: rest.to_string(),
                section: current_section.clone(),
                done: false,
            });
        } else if let Some(rest) = line.strip_prefix("- [x] ") {
            item_num += 1;
            let source_id = format!("{}.{}", section_num, item_num);
            tasks.push(ParsedTask {
                source_id,
                title: rest.to_string(),
                section: current_section.clone(),
                done: true,
            });
        }
    }

    tasks
}

/// Import tasks from a `tasks.md` file path into the database for a team.
/// Returns the number of tasks imported.
pub fn import_tasks(db: &Db, team_id: &str, tasks_path: &Path) -> Result<usize> {
    let content = fs::read_to_string(tasks_path)
        .with_context(|| format!("Failed to read tasks.md at {}", tasks_path.display()))?;

    let parsed = parse_tasks_md(&content);
    let count = parsed.len();

    let conn = db.writer.lock().unwrap();
    for task in &parsed {
        let status = if task.done {
            TaskStatus::Completed
        } else {
            TaskStatus::Pending
        };
        db::task::create(
            &conn,
            team_id,
            &task.source_id,
            &task.title,
            None,
            Some(&task.section),
            status,
        )?;
    }

    Ok(count)
}

// ── Exporter (4.2) ────────────────────────────────────────────────────────────

/// Render SQLite task state back to `tasks.md` format.
pub fn export_tasks(db: &Db, team_id: &str, tasks_path: &Path) -> Result<()> {
    let conn = db.readers.get()?;
    let tasks = db::task::list(&conn, team_id, db::task::TaskFilter::All)?;

    // Group by section
    let mut sections: Vec<String> = Vec::new();
    let mut section_tasks: std::collections::BTreeMap<String, Vec<db::task::Task>> =
        std::collections::BTreeMap::new();

    for task in tasks {
        let section = task.section.clone().unwrap_or_else(|| "Misc".to_string());
        section_tasks.entry(section.clone()).or_default().push(task);
        if !sections.contains(&section) {
            sections.push(section);
        }
    }

    let mut output = String::new();
    for section in &sections {
        output.push_str(&format!("## {}\n\n", section));
        if let Some(tasks) = section_tasks.get(section) {
            for task in tasks {
                let check = if task.status == TaskStatus::Completed {
                    "x"
                } else {
                    " "
                };
                output.push_str(&format!("- [{}] {}\n", check, task.title));
            }
        }
        output.push('\n');
    }

    fs::write(tasks_path, output)
        .with_context(|| format!("Failed to write tasks.md to {}", tasks_path.display()))?;
    Ok(())
}

// ── Reimport diff (4.3) ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDiff {
    pub added: Vec<ParsedTask>,
    pub removed: Vec<RemovedTask>,
    pub modified: Vec<ModifiedTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovedTask {
    pub source_id: String,
    pub title: String,
    pub current_status: String,
    pub attention_needed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModifiedTask {
    pub source_id: String,
    pub old_title: String,
    pub new_title: String,
}

/// Diff a new `tasks.md` against current SQLite state.
/// Returns a diff for lead review — does NOT apply changes.
pub fn diff_reimport(db: &Db, team_id: &str, tasks_path: &Path) -> Result<TaskDiff> {
    let content = fs::read_to_string(tasks_path)
        .with_context(|| format!("Failed to read tasks.md at {}", tasks_path.display()))?;
    let new_tasks = parse_tasks_md(&content);

    let conn = db.readers.get()?;
    let current_tasks = db::task::list(&conn, team_id, db::task::TaskFilter::All)?;

    let current_map: std::collections::HashMap<String, &db::task::Task> = current_tasks
        .iter()
        .map(|t| (t.source_id.clone(), t))
        .collect();
    let new_map: std::collections::HashMap<String, &ParsedTask> =
        new_tasks.iter().map(|t| (t.source_id.clone(), t)).collect();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();

    // Find added and modified
    for new_task in &new_tasks {
        if let Some(existing) = current_map.get(&new_task.source_id) {
            if existing.title != new_task.title {
                modified.push(ModifiedTask {
                    source_id: new_task.source_id.clone(),
                    old_title: existing.title.clone(),
                    new_title: new_task.title.clone(),
                });
            }
        } else {
            added.push(new_task.clone());
        }
    }

    // Find removed
    for current_task in &current_tasks {
        if !new_map.contains_key(&current_task.source_id) {
            let attention = current_task.status == db::task::TaskStatus::InProgress;
            removed.push(RemovedTask {
                source_id: current_task.source_id.clone(),
                title: current_task.title.clone(),
                current_status: current_task.status.as_str().to_string(),
                attention_needed: attention,
            });
        }
    }

    Ok(TaskDiff {
        added,
        removed,
        modified,
    })
}

/// Apply a confirmed diff to the database.
pub fn apply_diff(db: &Db, team_id: &str, diff: &TaskDiff) -> Result<()> {
    let conn = db.writer.lock().unwrap();
    for task in &diff.added {
        db::task::create(
            &conn,
            team_id,
            &task.source_id,
            &task.title,
            None,
            None,
            TaskStatus::Pending,
        )?;
    }
    // For modified: update title via raw SQL (not yet exposed in db::task)
    for modified in &diff.modified {
        conn.execute(
            "UPDATE tasks SET title = ?1 WHERE team_id = ?2 AND source_id = ?3",
            rusqlite::params![modified.new_title, team_id, modified.source_id],
        )?;
    }
    Ok(())
}
