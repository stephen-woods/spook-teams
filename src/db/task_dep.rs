use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

/// Add a dependency edge: `task_id` depends on `depends_on`.
/// Returns an error if adding this edge would create a cycle.
pub fn add_dependency(conn: &Connection, task_id: &str, depends_on: &str) -> Result<()> {
    // A cycle exists if depends_on already (transitively) depends on task_id.
    if has_path(conn, depends_on, task_id)? {
        bail!(
            "Adding dependency {} -> {} would create a cycle",
            task_id,
            depends_on
        );
    }
    conn.execute(
        "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on) VALUES (?1, ?2)",
        params![task_id, depends_on],
    )?;
    Ok(())
}

/// BFS to check whether `from` can reach `to` via the dependency graph.
fn has_path(conn: &Connection, from: &str, to: &str) -> Result<bool> {
    if from == to {
        return Ok(true);
    }
    let mut visited = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(from.to_string());
    visited.insert(from.to_string());

    while let Some(current) = queue.pop_front() {
        for dep in get_dependencies(conn, &current)? {
            if dep == to {
                return Ok(true);
            }
            if visited.insert(dep.clone()) {
                queue.push_back(dep);
            }
        }
    }
    Ok(false)
}

/// Get the list of task IDs that `task_id` directly depends on.
pub fn get_dependencies(conn: &Connection, task_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT depends_on FROM task_dependencies WHERE task_id = ?1")?;
    let deps = stmt
        .query_map(params![task_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(deps)
}

/// Get the list of task IDs that directly depend on `task_id`.
pub fn get_dependents(conn: &Connection, task_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT task_id FROM task_dependencies WHERE depends_on = ?1")?;
    let deps = stmt
        .query_map(params![task_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(deps)
}

/// Check whether all dependencies of `task_id` are completed.
pub fn is_unblocked(conn: &Connection, task_id: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM task_dependencies td
         JOIN tasks t ON t.id = td.depends_on
         WHERE td.task_id = ?1 AND t.status != 'completed'",
        params![task_id],
        |row| row.get(0),
    )?;
    Ok(count == 0)
}

/// Compute the set of tasks that become unblocked when `completed_task_id` is completed.
/// Returns IDs of tasks that were blocked and are now fully unblocked.
pub fn compute_newly_unblocked(conn: &Connection, completed_task_id: &str) -> Result<Vec<String>> {
    let dependents = get_dependents(conn, completed_task_id)?;
    let mut unblocked = Vec::new();
    for dep_id in dependents {
        if is_unblocked(conn, &dep_id)? {
            let status: Option<String> = conn
                .query_row(
                    "SELECT status FROM tasks WHERE id = ?1",
                    params![dep_id],
                    |row| row.get(0),
                )
                .optional()?;
            if status.as_deref() == Some("blocked") {
                unblocked.push(dep_id);
            }
        }
    }
    Ok(unblocked)
}
